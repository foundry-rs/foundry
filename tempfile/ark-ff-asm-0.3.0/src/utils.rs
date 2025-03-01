pub const RAX: &str = "%rax";
pub const RBX: &str = "%rbx";
pub const RCX: &str = "%rcx";
pub const RDX: &str = "%rdx";
// pub const RDI: &'static str = "%rdi";
// pub const RSI: &'static str = "%rsi";
pub const R: [&str; 8] = ["%r8", "%r9", "%r10", "%r11", "%r12", "%r13", "%r14", "%r15"];

macro_rules! reg {
    ($a_0:ident, $a_1:ident, $a:ident, $range:expr) => {
        let mut $a_0 = Vec::new();
        let mut $a_1 = Vec::new();
        for i in 0..$range {
            $a_0.push(format!("{}({})", i * 8, $a));
        }
        for i in 0..$range {
            $a_1.push(&*$a_0[i]);
        }
    };
}
