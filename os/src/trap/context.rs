//! Implementation of [`TrapContext`]
use riscv::register::sstatus::{self, Sstatus, SPP};
///初始化TrapContext，TrapContext用于保存一个任务在发生异常（trap）时的上下文信息，以便后续的恢复和继续执行
#[repr(C)]
#[derive(Debug)]
/// trap context structure containing sstatus, sepc and registers
pub struct TrapContext {
    /// General-Purpose Register x0-31
    pub x: [usize; 32],
    /// Supervisor Status Register
    pub sstatus: Sstatus,
    /// Supervisor Exception Program Counter：保存发生异常时程序计数器（PC）的值。即发生 trap 时，程序执行的地址
    pub sepc: usize,
    /// Token of kernel address space：保存内核地址空间的 Token，即内核页表的标识符（用于虚拟内存管理）
    pub kernel_satp: usize,
    /// Kernel stack pointer of the current application：保存当前任务的内核栈指针
    pub kernel_sp: usize,
    /// Virtual address of trap handler entry point in kernel：保存内核中的异常处理程序（trap_handler）的虚拟地址。异常发生时将跳转到这个地址执行。
    pub trap_handler: usize,
}

impl TrapContext {
    /// put the sp(stack pointer) into x\[2\] field of TrapContext：将 sp（栈指针）设置到 TrapContext 中的 x[2] 寄存器。x[2] 在 RISC-V 中被保留作为栈指针寄存器，用来指向当前函数的栈帧。
    pub fn set_sp(&mut self, sp: usize) {
        self.x[2] = sp;
    }
    /// init the trap context of an application：用于初始化应用程序的 TrapContext
    pub fn app_init_context(
        entry: usize,//入口
        sp: usize,//栈指针
        kernel_satp: usize,//页表地址
        kernel_sp: usize,//内核栈栈指针
        trap_handler: usize,//异常的虚拟地址
    ) -> Self {
        let mut sstatus = sstatus::read();
        // set CPU privilege to User after trapping back
        sstatus.set_spp(SPP::User);//设置 CPU 特权级为用户模式
        let mut cx = Self {
            x: [0; 32],
            sstatus,
            sepc: entry,  // entry point of app
            kernel_satp,  // addr of page table
            kernel_sp,    // kernel stack
            trap_handler, // addr of trap_handler function
        };
        cx.set_sp(sp); // app's user stack pointer
        cx // return initial Trap Context of app
    }
}
