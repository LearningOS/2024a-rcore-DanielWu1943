//!Implementation of [`TaskManager`]
//use core::future::Ready;

use super::{TaskControlBlock, TaskStatus};
use crate::sync::UPSafeCell;
use crate::config::BIG_STRIDE;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,//发现用智能指针来控制队列，因为TCB经常需要放入和取出，智能指针进行移动则没有多少开销
}

/// A simple FIFO scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }
    /// Take a process out of the ready queue
    /// 从ready queue里取出一个任务，现在是把最前面的取出来，我们要修改成：从当前 runnable 态的进程中选择 stride 最小的进程调度
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        //self.ready_queue.pop_front()
        let mut min_tride = 0x7FFF_FFFF;
        let mut min_idx = 0;
        //遍历 ready queue，找到最小stride的进程
        for (idx, task) in self.ready_queue.iter().enumerate() {
            let inner = task.inner_exclusive_access();
            if inner.get_status() == TaskStatus::Ready{
                if inner.stride < min_tride {
                    min_idx = idx;
                    min_tride = inner.stride;
                }
            }
        }
        //更新调度任务的stride
        if let Some(task) = self.ready_queue.get(min_idx) {
            let mut inner = task.inner_exclusive_access();
            inner.stride += BIG_STRIDE / inner.priority;
        }
        self.ready_queue.remove(min_idx)
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}
