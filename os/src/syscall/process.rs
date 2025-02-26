//! Process management syscalls
use crate::config::PAGE_SIZE;
use crate::timer::{get_time_us, get_time_ms};
use crate::{sync::UPSafeCell, trap::trap_handler};
//use alloc::task;
use alloc::{sync::Arc, vec::Vec};
//use xmas_elf::P32;
//use riscv::addr::VirtAddr;
use crate::mm::{translated_byte_buffer, VirtAddr, KERNEL_SPACE, MapPermission};

use crate::{
    config::{MAX_SYSCALL_NUM, TRAP_CONTEXT_BASE, MAXVA}, loader::get_app_data_by_name, mm::{translated_refmut, translated_str, MemorySet, VPNRange},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next, pid_alloc,
        suspend_current_and_run_next, TaskContext, TaskControlBlock, TaskStatus, kstack_alloc,
        TaskControlBlockInner, get_current_task_page_table, create_new_map_area, unmap_consecutive_area,
        get_current_task_time_cost, get_current_task_syscall_times, get_current_task_status,
    },
    trap::TrapContext,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// Task information
#[allow(dead_code)]
pub struct TaskInfo {
    /// Task status in it's life cycle
    status: TaskStatus,
    /// The numbers of syscall called by task
    syscall_times: [u32; MAX_SYSCALL_NUM],
    /// Total running time of task
    time: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

///fork：
pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

/// 字符串 path 给出了要加载的可执行文件的名字
/// translated_str：从path开始逐字节查页表直到发现一个 \0 为止
/// 1. 先获取当前user的token，即页表 page table；2. 根据page table和 path （APP对应的虚拟地址的起始位置）开始读取，即将虚拟地址path转换成虚拟地址对应的物理地址中的数据；
/// 3. 找到path.as_str()（APP name）对应的数据data；4. 调用task.exec(data);处理子进程的数据。
pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        task.exec(data);
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!("kernel::pid[{}] sys_waitpid [{}]", current_task().unwrap().pid.0, pid);
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_get_time",
        current_task().unwrap().pid.0
    );
    let us = get_time_us();
    let ref timeval = TimeVal{
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let src_ptr = timeval as *const TimeVal;
    let dst_ptr = translated_byte_buffer(current_user_token(), _ts as *const u8, core::mem::size_of::<TimeVal>());
    for (idx, dst) in dst_ptr.into_iter().enumerate() {
        let unit_len = dst.len();
        unsafe {
            dst.copy_from_slice(core::slice::from_raw_parts(
                src_ptr.wrapping_byte_add(idx * unit_len) as *const u8,
                unit_len));
        }
    }
    0

}

/// YOUR JOB: Finish sys_task_info to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TaskInfo`] is splitted by two pages ?
pub fn sys_task_info(_ti: *mut TaskInfo) -> isize {
    trace!(
        "kernel:pid[{}] sys_task_info",
        current_task().unwrap().pid.0
    );
    let ref task_info = TaskInfo {
        status: get_current_task_status(),
        syscall_times: get_current_task_syscall_times(),
        time: get_current_task_time_cost(),
    };
    println!("[kernel]: time {} syscall_time {}", task_info.time, task_info.syscall_times[super::SYSCALL_GET_TIME]);
    let src_ptr = task_info as *const TaskInfo;//virtual add
    let dst_ptr = translated_byte_buffer(current_user_token(), _ti as *const u8, core::mem::size_of::<TaskInfo>());//current user token: the page table of current user
    for (idx, dst) in dst_ptr.into_iter().enumerate() {
        let unit_len = dst.len();
        unsafe {
            dst.copy_from_slice(core::slice::from_raw_parts(src_ptr.wrapping_byte_add(idx * unit_len) as *const u8, unit_len));
        }
    }
    0
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_mmap",
        current_task().unwrap().pid.0
    );
    if _start % PAGE_SIZE != 0 ||
    _port & !0x07 != 0 ||
    _port & 0x07 == 0||
    _start >= MAXVA {
        return -1;
    }
    let start_vpn = VirtAddr::from(_start).floor();
    let end_vpn = VirtAddr::from(_start + _len).ceil();
    let vpns = VPNRange::new(start_vpn, end_vpn);//创建从start_vpn到end_vpn的VPN列表
    for vpn in vpns {
        if let Some(pte) = get_current_task_page_table(vpn) {
            if pte.is_valid() {
                return -1;
            }
        }
    }
    create_new_map_area(
        start_vpn.into(),
        end_vpn.into(),
        MapPermission::from_bits_truncate((_port << 1) as u8) | MapPermission::U
    );
    0

}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    if _start >= MAXVA || _start % PAGE_SIZE != 0 {
        return -1;
    }
    // avoid undefined situation
    let mut mlen = _len;
    if _start > MAXVA - _len {
        mlen = MAXVA - _start;
    }
    unmap_consecutive_area(_start, mlen)
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
/// 新建子进程，使其执行目标程序
/// 成功返回子进程id，否则返回 -1
/// 1. 获取当前任务，使用fork创建一个新的task（子进程）；2. 对创建的新 task 做 exec 如果成功返回子进程id
/// 我的问题在于：1、没有给child分配 new_pid，这new_task的pid和原本的还是一样的；2、步骤不完全
/// 1. 获取当前任务的token和elf；2. 分配一个新的pid和kernel stack；3. 初始化子进程的TCB；4. 准备子进程的trap context
pub fn sys_spawn(_path: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_spawn",
        current_task().unwrap().pid.0
    );
    let current_task = current_task().unwrap();
    let token = current_user_token();
    let mut parent_inner = current_task.inner_exclusive_access();
    let path = translated_str(token, _path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(data);
        let trap_cx_ppn = memory_set.translate(VirtAddr::from(TRAP_CONTEXT_BASE).into()).unwrap().ppn();
        // Alloc PID to child
        let new_pid = pid_alloc();
        // Alloc a Kernel Stack
        let kernel_stack = kstack_alloc();
        let kernel_stack_top = kernel_stack.get_top();
        let task_control_block = Arc::new(TaskControlBlock{
            pid: new_pid,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ppn,
                    base_size: parent_inner.base_size,
                    task_cx: TaskContext::goto_trap_return(kernel_stack_top),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    parent: Some(Arc::downgrade(&current_task)),
                    children: Vec::new(),
                    exit_code: 0,
                    heap_bottom: parent_inner.heap_bottom,
                    program_brk: parent_inner.program_brk,
                    stride: 0,
                    priority: 16,
                    syscall_times: [0; MAX_SYSCALL_NUM],
                    kernel_time: 0,
                    user_time: 0,
                    checkpoint: get_time_ms(),
                })
            },
        });
        //add child
        parent_inner.children.push(task_control_block.clone());
        let trap_cx = task_control_block.inner_exclusive_access().get_trap_cx();
        *trap_cx = TrapContext::app_init_context(entry_point, user_sp, KERNEL_SPACE.exclusive_access().token(), kernel_stack_top, trap_handler as usize);
        let pid = task_control_block.pid.0 as isize;
        add_task(task_control_block);
        pid
    } else {
        -1
    }
    
}

// YOUR JOB: Set task priority.
/// input: _prio >= 2
/// 设置当前优先级为_prio
pub fn sys_set_priority(_prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority",
        current_task().unwrap().pid.0
    );
    if _prio <= 1{
        return -1;
    }
    let task = current_task().unwrap();
    task.inner_exclusive_access().set_priority(_prio as u64);
    _prio
}
