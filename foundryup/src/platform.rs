use crate::errors::FoundryupError;
use std::env;

/// Types of supported platforms (build in ci)
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum Platform {
    LinuxAmd64,
    LinuxAarch64,
    MacOsAmd64,
    MacOsAarch64,
    // TODO enable once working windows builds
    // WindowsAmd64,
    Unsupported,
}

impl Platform {
    /// Detects the `Platform` of the current OS
    pub fn current() -> Platform {
        match (env::consts::OS, env::consts::ARCH) {
            ("linux", "x86_64") => Platform::LinuxAmd64,
            ("linux", "aarch64") => Platform::LinuxAarch64,
            ("macos", "x86_64") => Platform::MacOsAmd64,
            ("macos", "aarch64") => Platform::MacOsAarch64,
            // ("windows", "x86_64") => Platform::WindowsAmd64,
            _ => Platform::Unsupported,
        }
    }

    /// Ensures that the platform is supported by foundry, returns an error otherwise
    pub fn ensure_supported(self) -> Result<Self, FoundryupError> {
        if self == Platform::Unsupported {
            Err(FoundryupError::UnsupportedPlatform {
                os: env::consts::OS,
                arch: env::consts::ARCH,
            })
        } else {
            Ok(self)
        }
    }

    pub fn platform_name(&self) -> &'static str {
        match self {
            Platform::LinuxAmd64 | Platform::LinuxAarch64 => "linux",
            Platform::MacOsAmd64 | Platform::MacOsAarch64 => "darwin",
            Platform::Unsupported => "unsupported",
        }
    }

    pub fn arch_name(&self) -> &'static str {
        match self {
            Platform::LinuxAmd64 => "amd64",
            Platform::LinuxAarch64 => "arm64",
            Platform::MacOsAmd64 => "arm64",
            Platform::MacOsAarch64 => "amd64",
            Platform::Unsupported => "unsupported",
        }
    }
}
