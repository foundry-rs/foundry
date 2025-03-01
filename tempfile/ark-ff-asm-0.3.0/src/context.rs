use std::collections::HashMap;

pub const REG_CLOBBER: [&str; 8] = ["r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"];

#[derive(Clone)]
pub struct Context {
    ctx_string: String,
    declarations: HashMap<String, Declare>,
    declaration_vec: Vec<Declare>,
    clobbers: Vec<String>,
}

#[derive(Clone)]
struct Declare {
    ty: String,
    var: String,
    pos: usize,
    token: String,
}

impl Context {
    pub fn new() -> Self {
        Context {
            ctx_string: String::new(),
            declarations: HashMap::new(),
            declaration_vec: Vec::new(),
            clobbers: Vec::new(),
        }
    }

    fn append(&mut self, other: &str) {
        self.ctx_string += other;
    }

    pub fn get_string(&self) -> String {
        self.ctx_string.clone()
    }

    pub fn get(self, id: &str) -> String {
        self.declarations.get(id).unwrap().token.clone()
    }

    pub fn try_get(self, id: &str, fallback_id: &str) -> String {
        self.declarations
            .get(id)
            .map(|dec| dec.token.clone())
            .unwrap_or_else(|| self.get(fallback_id))
    }

    pub fn add_declaration(&mut self, id: &str, ty: &str, var: &str) {
        self.declarations.insert(
            id.to_string(),
            Declare {
                ty: ty.to_string(),
                var: var.to_string(),
                pos: self.declarations.len(),
                token: format!("${}", self.declarations.len()),
            },
        );
        self.declaration_vec.push(Declare {
            ty: ty.to_string(),
            var: var.to_string(),
            pos: self.declaration_vec.len(),
            token: format!("${}", self.declaration_vec.len()),
        });
    }

    pub fn add_buffer(&mut self, extra_reg: usize) {
        self.append(&format!(
            "
                    let mut spill_buffer = core::mem::MaybeUninit::<[u64; {}]>::uninit();",
            extra_reg
        ));
    }

    pub fn add_llvm_asm(&mut self, ctx_string: String) {
        self.append(&format!(
            "
                    unsafe {{
                        llvm_asm!({}
                            :
                            :",
            ctx_string
        ));
    }

    pub fn add_clobber_from_vec(&mut self, clobbers: Vec<&str>) {
        for clobber in clobbers {
            self.clobbers.push(format!(" \"{}\"", clobber));
        }
    }

    pub fn add_clobber(&mut self, clobber: &str) {
        self.clobbers.push(format!(" \"{}\"", clobber));
    }

    pub fn build(&mut self) {
        for i in 0..self.declarations.len() {
            let dec = &self.declaration_vec[i];
            let last = i == self.declarations.len() - 1;
            let dec = &format!(
                "
                            \"{}\"({}){}      // {}",
                dec.ty,
                dec.var,
                if last { "" } else { "," },
                dec.pos
            );
            self.append(dec);
        }
        let clobbers = self.clobbers.join(",");
        self.append(&format!(
            "
                            : {}
                        );
                    }}
                ",
            clobbers
        ));
    }
}
