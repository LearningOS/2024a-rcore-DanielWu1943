//! Process management syscalls
use crate::{
    config::MAX_SYSCALL_NUM,
    task::{exit_current_and_run_next, suspend_current_and_run_next, TaskStatus, TASK_MANAGER},
    timer::get_time_us,
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
    trace!("[kernel] Application exited with code {}", exit_code);
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// get time with second and microsecond
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    unsafe {
        *ts = TimeVal {
            sec: us / 1_000_000,
            usec: us % 1_000_000,
        };
    }
    0
}

/// YOUR JOB: Finish sys_task_info to pass testcases
/// 获得任务控制块相关信息（任务状态）、任务使用的系统调用及调用次数、系统调用时刻距离任务第一次被调度时刻的时长（单位ms）
/*
unsafe：因为RUST的所有权和借用规则通常会防止直接修改指针所指向的数据，而 unsafe 允许绕过这些规则。
因为我们是通过TaskManager来管理任务的，然而TaskManager中没有给出获取当前任务的函数，即get current task，那我们就要自己实现。
*/
pub fn sys_task_info(_ti: *mut TaskInfo) -> isize {
    trace!("kernel: sys_task_info");
    // 获取当前任务
    let task = TASK_MANAGER.get_current_task();
    
    // 获取系统调用相关信息
    let syscall_times = task.syscall_times;
    
    // 获取任务状态
    let task_status = task.task_status;
    let time_sys_call = get_time_us();//系统调用的时间（当前时间）

    // 计算从任务第一次调度到当前的时间差（单位：毫秒）
    //由于start_time是Option类型，先解包
    if let Some(start_time) = task.start_time {
        let elapsed_time_ms = (time_sys_call - start_time) / 1000; // 计算时间差（单位：毫秒）
        
        // 填充任务信息到 _ti 结构体中
        unsafe {
            (*_ti).status = task_status;          // 任务状态
            (*_ti).syscall_times = syscall_times; // 系统调用次数
            (*_ti).time = elapsed_time_ms;       // 系统调用时刻到任务第一次调度的时长
        }
    } else {
        // 如果 start_time 为 None，任务尚未开始，进行相应处理
        return -1; // 根据实际需求决定返回值，可以选择返回错误码
    }

    0 // 返回0表示成功
}
