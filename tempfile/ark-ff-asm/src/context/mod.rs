mod data_structures;
pub use data_structures::*;

#[derive(Clone)]
pub struct Context<'a> {
    assembly_instructions: Vec<String>,
    declarations: Vec<Declaration<'a>>,
    used_registers: Vec<Register<'a>>,
}

impl<'a> Context<'a> {
    pub const RAX: Register<'static> = Register("rax");
    pub const RSI: Register<'static> = Register("rsi");
    pub const RCX: Register<'static> = Register("rcx");
    pub const RDX: Register<'static> = Register("rdx");

    pub const R: [Register<'static>; 8] = [
        Register("r8"),
        Register("r9"),
        Register("r10"),
        Register("r11"),
        Register("r12"),
        Register("r13"),
        Register("r14"),
        Register("r15"),
    ];

    pub fn new() -> Self {
        Self {
            assembly_instructions: Vec::new(),
            declarations: Vec::new(),
            used_registers: Vec::new(),
        }
    }

    fn find(&self, name: &str) -> Option<&Declaration<'_>> {
        self.declarations.iter().find(|item| item.name == name)
    }

    fn append(&mut self, other: &str) {
        self.assembly_instructions.push(format!("\"{}\",", other));
    }

    fn instructions_to_string(&self) -> String {
        self.assembly_instructions.join("\n")
    }

    fn get_decl_name(&self, name: &str) -> Option<&Declaration<'_>> {
        self.find(name)
    }

    pub fn get_decl(&self, name: &str) -> Declaration<'_> {
        *self.get_decl_name(name).unwrap()
    }

    pub fn get_decl_with_fallback(&self, name: &str, fallback_name: &str) -> Declaration<'_> {
        self.get_decl_name(name)
            .copied()
            .unwrap_or_else(|| self.get_decl(fallback_name))
    }

    pub fn add_declaration(&mut self, name: &'a str, expr: &'a str) {
        let declaration = Declaration { name, expr };
        self.declarations.push(declaration);
    }

    pub fn add_buffer(&mut self, extra_reg: usize) {
        self.append(&format!(
            "let mut spill_buffer = core::mem::MaybeUninit::<[u64; {}]>::uninit();",
            extra_reg
        ));
    }

    pub fn add_asm(&mut self, asm_instructions: &[String]) {
        for instruction in asm_instructions {
            self.append(instruction)
        }
    }

    pub fn add_clobbers(&mut self, clobbers: impl Iterator<Item = Register<'a>>) {
        for clobber in clobbers {
            self.add_clobber(clobber)
        }
    }

    pub fn add_clobber(&mut self, clobber: Register<'a>) {
        self.used_registers.push(clobber);
    }

    pub fn build(self) -> String {
        let declarations: String = self
            .declarations
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>()
            .join("\n");
        let clobbers = self
            .used_registers
            .iter()
            .map(|l| format!("out({}) _,", l))
            .collect::<Vec<String>>()
            .join("\n");
        let options = "options(att_syntax)".to_string();
        let assembly = self.instructions_to_string();
        [
            "unsafe {".to_string(),
            "ark_std::arch::asm!(".to_string(),
            assembly,
            declarations,
            clobbers,
            options,
            ")".to_string(),
            "}".to_string(),
        ]
        .join("\n")
    }
}
