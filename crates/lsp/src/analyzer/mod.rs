use forge_lint::{linter::Linter, sol::SolidityLinter};
use foundry_cli::opts::{BuildOpts, solar_pcx_from_build_opts};
use foundry_compilers::{FileFilter, solc::SolcLanguage};
use foundry_config::{Config, SkipBuildFilters};
use solar_interface::{
    Session,
    diagnostics::{self, DiagCtxt, JsonEmitter},
    source_map::SourceMap,
};
use solar_sema::{GcxWrapper, hir, thread_local::ThreadLocal};
use std::{
    io, ptr,
    sync::{Arc, Mutex},
};
use tower_lsp::lsp_types::{Diagnostic, Url};

#[cfg(test)]
pub(crate) mod test_utils;

pub mod code_actions;
pub mod lint;

// Ownable GcxWrapper (self-referential).
struct OwnableGcxWrapper {
    /// The owner of the HIR data.
    hir_arena: ThreadLocal<hir::Arena>,
    /// A reference to the context, which itself borrows hir_arena.
    /// We lie with 'static cause we manually that the lifetime is valid.
    wrapper: GcxWrapper<'static>,
}

impl OwnableGcxWrapper {
    /// Provides safe, scoped access to the gcx with its proper lifetime.
    fn gcx_wrapper(&self) -> &GcxWrapper<'_> {
        // The lifetime of the returned GcxWrapper should be tied to the lifetime
        // of `&self`. We are "downcasting" the 'static lifetime to the shorter,
        // correct lifetime of the borrow.
        //
        // This is safe ONLY because we know `self.gcx` contains references into
        // `self.hir_arena`, and `self.hir_arena` is guaranteed to live as long
        // as `self`. Therefore, the data is valid for the duration of the `&self` borrow.
        unsafe { std::mem::transmute::<&GcxWrapper<'static>, &GcxWrapper<'_>>(&self.wrapper) }
    }
}

/// A Forge project, analyzed by Solar.
pub struct Analyzer {
    pub config: Config,
    pub opts: BuildOpts,
    pub sess: Session,
    pub linter: SolidityLinter,
    // Stored in a Box to have a stable location on the heap.
    ogcxw: Option<Box<OwnableGcxWrapper>>,
    diagnostics: Arc<Mutex<Vec<u8>>>,
}

impl Analyzer {
    /// Creates a new, empty analyzer project.
    pub fn new(config: Config, opts: BuildOpts) -> Self {
        let linter = SolidityLinter::new(config.project_paths());
        let mut builder = Session::builder();
        let map = Arc::<SourceMap>::default();
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let json_emitter = JsonEmitter::new(Box::new(SharedBuffer(buffer.clone())), map.clone())
            .rustc_like(true)
            .ui_testing(false);

        builder = builder.dcx(DiagCtxt::new(Box::new(json_emitter))).source_map(map);

        let mut sess = builder.build();
        sess.dcx = sess.dcx.set_flags(|flags| flags.track_diagnostics = false);

        Self { config, opts, sess, linter, ogcxw: None, diagnostics: buffer }
    }

    pub fn analyze(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let skip = SkipBuildFilters::new(self.config.skip.clone(), self.config.root.clone());
        let input_files = self
            .config
            .project_paths::<SolcLanguage>()
            .input_files_iter()
            .filter(|p| skip.is_match(p))
            .collect::<Vec<_>>();

        // Clear the diagnostics' buffer before running the linter.
        self.diagnostics.lock().unwrap().clear();
        let pcx = solar_pcx_from_build_opts(
            &self.sess,
            &self.opts,
            self.config.project().ok().as_ref(),
            Some(&input_files),
        )?;
        self.linter.early_lint(&input_files, pcx);

        let pcx = solar_pcx_from_build_opts(
            &self.sess,
            &self.opts,
            self.config.project().ok().as_ref(),
            Some(&input_files),
        )?;

        let ogcxw_res = self.sess.enter_parallel(|| -> Result<Box<OwnableGcxWrapper>, _> {
            // Allocate uninitialized memory on the heap for the entire `Analysis` struct, and get a
            // raw pointer to its memory location;
            let mut b = Box::new_uninit();
            let ptr: *mut OwnableGcxWrapper = b.as_mut_ptr();

            // Create the `Arena` (owner) and manually write it into its memory location.
            let hir_arena = solar_sema::thread_local::ThreadLocal::new();
            let arena_ptr = unsafe { ptr::addr_of_mut!((*ptr).hir_arena) };
            unsafe { ptr::write(arena_ptr, hir_arena) };

            // Now that the owner is stable, create the `Gcx<'arena>` by borrowing from it. This
            // is safe because the `hir_arena` won't be moved again.
            let arena_ref = unsafe { &*arena_ptr };
            match pcx.parse_and_lower(arena_ref) {
                Ok(Some(gcx_wrapper)) => {
                    // The lifetime of gcx_wrapper is tied to `arena_ref`, so it is safe to
                    // transmute it to 'static.
                    let static_gcx: GcxWrapper<'static> =
                        unsafe { std::mem::transmute(gcx_wrapper) };

                    // Manually write the `Gcx<'arena>` into its memory location.
                    let gcx_ptr = unsafe { ptr::addr_of_mut!((*ptr).wrapper) };
                    unsafe { ptr::write(gcx_ptr, static_gcx) };

                    // All fields are initialized. It is safe to transition the box to `Analysis`.
                    Ok(unsafe { b.assume_init() })
                }
                // Handle failure cases by manually dropping the `hir_arena`.
                Ok(None) => {
                    unsafe { ptr::drop_in_place(arena_ptr) };
                    Err(diagnostics::ErrorGuaranteed::new_unchecked())
                }
                Err(e) => {
                    unsafe { ptr::drop_in_place(arena_ptr) };
                    Err(e)
                }
            }
        });

        if let Ok(ogcxw) = ogcxw_res {
            self.linter.process_parallel_sources_hir(&input_files, ogcxw.gcx_wrapper().get());
            self.ogcxw = Some(ogcxw);
        } else {
            self.ogcxw = None;
        }

        Ok(())
    }

    /// Parses the diagnostics from the buffer.
    pub fn diagnostics(&self) -> serde_json::Value {
        let bytes = self.diagnostics.lock().unwrap();
        let stderr_str = String::from_utf8_lossy(&bytes);

        // Parse JSON output line by line
        let mut diagnostics = Vec::new();
        for line in stderr_str.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                diagnostics.push(value);
            }
        }
        serde_json::Value::Array(diagnostics)
    }

    /// Parses the diagnostics from the buffer and converts them to an lsp-compatible type.
    pub fn get_lint_diagnostics(&self, file: &Url) -> Option<Vec<Diagnostic>> {
        let path = file.to_file_path().ok()?;
        let path_str = path.to_str()?;
        Some(lint::lint_output_to_diagnostics(&self.diagnostics(), path_str))
    }
}

#[derive(Clone)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}
