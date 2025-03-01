pub(super) mod java {
    use crate::flamegraph::color::BasicPalette;

    /// Handle both annotations (_[j], _[i], ...; which are
    /// accurate), as well as input that lacks any annotations, as
    /// best as possible. Without annotations, we get a little hacky
    /// and match on java|org|com, etc.
    pub(in super::super) fn resolve(name: &str) -> BasicPalette {
        if name.ends_with(']') {
            if let Some(ai) = name.rfind("_[") {
                if name[ai..].len() == 4 {
                    match &name[ai + 2..ai + 3] {
                        // kernel annotation
                        "k" => return BasicPalette::Orange,
                        // inline annotation
                        "i" => return BasicPalette::Aqua,
                        // jit annotation
                        "j" => return BasicPalette::Green,
                        _ => {}
                    }
                }
            }
        }

        let java_prefix = name.strip_prefix('L').unwrap_or(name);

        if name.contains("::") || name.starts_with("-[") || name.starts_with("+[") {
            // C++ or Objective C
            BasicPalette::Yellow
        } else if java_prefix.contains('/')
            || (java_prefix.contains('.') && !java_prefix.starts_with('['))
            || match java_prefix.chars().next() {
                Some(c) => c.is_ascii_uppercase(),
                _ => false,
            }
        {
            // Java
            BasicPalette::Green
        } else {
            // system
            BasicPalette::Red
        }
    }
}

pub(super) mod perl {
    use crate::flamegraph::color::BasicPalette;

    pub(in super::super) fn resolve(name: &str) -> BasicPalette {
        if name.ends_with("_[k]") {
            BasicPalette::Orange
        } else if name.contains("Perl") || name.contains(".pl") {
            BasicPalette::Green
        } else if name.contains("::") {
            BasicPalette::Yellow
        } else {
            BasicPalette::Red
        }
    }
}

pub(super) mod python {
    use crate::flamegraph::color::BasicPalette;

    fn split_any_path(path: &str) -> impl Iterator<Item = &str> {
        path.split(|c| c == '/' || c == '\\')
    }

    pub(in super::super) fn resolve(name: &str) -> BasicPalette {
        if split_any_path(name).any(|part| part == "site-packages") {
            BasicPalette::Aqua
        } else if split_any_path(name).any(|part| {
            part.strip_prefix("python")
                .or_else(|| part.strip_prefix("Python"))
                .map_or(false, |version| {
                    version.chars().all(|c| c.is_ascii_digit() || c == '.')
                })
        }) || name.starts_with("<built-in")
            || name.starts_with("<method")
            || name.starts_with("<frozen")
        {
            // stdlib
            BasicPalette::Yellow
        } else {
            BasicPalette::Red
        }
    }
}

pub(super) mod js {
    use crate::flamegraph::color::BasicPalette;

    pub(in super::super) fn resolve(name: &str) -> BasicPalette {
        if !name.is_empty() && name.trim().is_empty() {
            return BasicPalette::Green;
        } else if name.ends_with("_[k]") {
            return BasicPalette::Orange;
        } else if name.ends_with("_[j]") {
            if name.contains('/') {
                return BasicPalette::Green;
            } else {
                return BasicPalette::Aqua;
            }
        } else if name.contains("::") {
            return BasicPalette::Yellow;
        } else if name.contains(':') {
            return BasicPalette::Aqua;
        } else if let Some(ai) = name.find('/') {
            if name[ai..].contains("node_modules/") {
                return BasicPalette::Purple;
            } else if name[ai..].contains(".js") {
                return BasicPalette::Green;
            }
        }

        BasicPalette::Red
    }
}

pub(super) mod wakeup {
    use crate::flamegraph::color::BasicPalette;

    pub(in super::super) fn resolve(_name: &str) -> BasicPalette {
        BasicPalette::Aqua
    }
}

pub(super) mod rust {
    use crate::flamegraph::color::BasicPalette;

    pub(in super::super) fn resolve(name: &str) -> BasicPalette {
        let name = name.split_once('`').map(|(_, after)| after).unwrap_or(name);
        if name.starts_with("core::")
            || name.starts_with("std::")
            || name.starts_with("alloc::")
            || (name.starts_with("<core::")
                // Rust user-defined async functions are desugared into
                // GenFutures so we don't want to include those as Rust
                // system functions
                && !name.starts_with("<core::future::from_generator::GenFuture<T>"))
            || name.starts_with("<std::")
            || name.starts_with("<alloc::")
        {
            // Rust system functions
            BasicPalette::Orange
        } else if name.contains("::") {
            // Rust user functions.
            // Although this will generate false positives for e.g. C++ code
            // used with Rust, the intention is to color code from user
            // crates and dependencies differently than Rust system code.
            BasicPalette::Aqua
        } else {
            // Non-Rust functions
            BasicPalette::Yellow
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::flamegraph::color::BasicPalette;

    struct TestData {
        input: String,
        output: BasicPalette,
    }

    #[test]
    fn java_mod_resolves() {
        use super::java::resolve;

        let test_names = [
            TestData {
                input: String::from("_[k]"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("_[j]_[k]"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("_[]_[k]"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("_[j]"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("_[k]_[j]"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("_[]_[j]"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("_[i]"),
                output: BasicPalette::Aqua,
            },
            TestData {
                input: String::from("_[j]_[i]"),
                output: BasicPalette::Aqua,
            },
            TestData {
                input: String::from("_[]_[i]"),
                output: BasicPalette::Aqua,
            },
            TestData {
                input: String::from("_[j]_[]"),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from("_[j]_[jj]"),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from("_[jk]"),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from("_[i]blah"),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from("java/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("java/somestuff"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("javax/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("javax/somestuff"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("jdk/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("jdk/somestuff"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("net/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("net/somestuff"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("org/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("org/somestuff"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("com/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("com/somestuff"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("io/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("io/somestuff"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("sun/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("sun/somestuff"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("Ljava/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("Ljavax/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("Ljdk/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("Lnet/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("Lorg/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("Lcom/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("Lio/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("Lsun/"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("jdk/_[ki]"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("jdk/::[ki]"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("Ajdk/_[ki]"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("Ajdk/::[ki]"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("jdk::[ki]"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("::[ki]"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("::"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("some::st_[jk]uff"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("jdk"),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from("Ljdk"),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from(" "),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from(""),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from("something"),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from("some:thing"),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from("scala.tools.nsc.Global$Run.compile"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("sbt.execute.work"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("org.scalatest.Suit.run"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("Compile"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("-[test]"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("+[test]"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("[test.event]"),
                output: BasicPalette::Red,
            },
        ];

        for item in test_names.iter() {
            let resolved_color = resolve(&item.input);
            assert_eq!(resolved_color, item.output)
        }
    }
    #[test]
    fn perl_mod_resolves() {
        use super::perl::resolve;

        let test_names = [
            TestData {
                input: String::from(" "),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from(""),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from("something"),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from("somethingpl"),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from("something/_[k]"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("something_[k]"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("some::thing"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("some/ai.pl"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("someai.pl"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("something/Perl"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("somethingPerl"),
                output: BasicPalette::Green,
            },
        ];

        for item in test_names.iter() {
            let resolved_color = resolve(&item.input);
            assert_eq!(resolved_color, item.output)
        }
    }

    #[test]
    fn python_returns_correct() {
        use super::python::resolve;

        let test_names = [
            TestData {
                input: String::from("<frozen importlib._bootstrap>:_load_unlocked:680"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("<built-in method time.sleep>"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("<method 'append' of 'list' objects>"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from(".venv/lib/python3.9/time.py:12"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("C:/Users/User/AppData/Local/Programs/Python/Python39/lib/concurrent/futures/thread.py"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("C:\\Users\\User\\AppData\\Local\\Programs\\Python\\Python39\\lib\\concurrent\\futures\\thread.py"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("my_file.py:55"),
                output: BasicPalette::Red,
            },
            TestData {
                input: String::from(".venv/lib/python3.9/site-packages/package/file.py:12"),
                output: BasicPalette::Aqua,
            },
        ];

        for item in test_names.iter() {
            let resolved_color = resolve(&item.input);
            assert_eq!(resolved_color, item.output)
        }
    }

    #[test]
    fn js_returns_correct() {
        use super::js;

        let test_data = [
            TestData {
                input: String::from(" "),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("something_[k]"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("something/_[j]"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("something_[j]"),
                output: BasicPalette::Aqua,
            },
            TestData {
                input: String::from("some::thing"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("some:thing"),
                output: BasicPalette::Aqua,
            },
            TestData {
                input: String::from("some/ai.js"),
                output: BasicPalette::Green,
            },
            TestData {
                input: String::from("project/node_modules/dep/index.js"),
                output: BasicPalette::Purple,
            },
            TestData {
                input: String::from("someai.js"),
                output: BasicPalette::Red,
            },
        ];
        for elem in test_data.iter() {
            let result = js::resolve(&elem.input);
            assert_eq!(result, elem.output);
        }
    }

    #[test]
    fn rust_returns_correct() {
        use super::rust;

        let test_names = [
            TestData {
                input: String::from("some::not_rust_system::mod"),
                output: BasicPalette::Aqua,
            },
            TestData {
                input: String::from("core::mod"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("std::mod"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("alloc::mod"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("something_else"),
                output: BasicPalette::Yellow,
            },
            TestData {
                input: String::from("<alloc::boxed::Box<F,A> as something::else"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("<core::something as something::else"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from(
                    "<core::future::from_generator::GenFuture<T> as something::else",
                ),
                output: BasicPalette::Aqua,
            },
            TestData {
                input: String::from("<std::something something::else"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("my-app`std::sys::unix::thread::Thread::new::thread_start"),
                output: BasicPalette::Orange,
            },
            TestData {
                input: String::from("my-app`foobar::extent::Extent::write"),
                output: BasicPalette::Aqua,
            },
        ];
        for elem in test_names.iter() {
            let result = rust::resolve(&elem.input);
            assert_eq!(result, elem.output);
        }
    }
}
