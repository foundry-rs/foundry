use addr2line::Context;
use fallible_iterator::FallibleIterator;
use findshlibs::{IterationControl, SharedLibrary, TargetSharedLibrary};
use object::Object;
use std::borrow::Cow;
use std::fs::File;
use std::sync::Arc;

fn find_debuginfo() -> memmap2::Mmap {
    let path = std::env::current_exe().unwrap();
    let file = File::open(&path).unwrap();
    let map = unsafe { memmap2::Mmap::map(&file).unwrap() };
    let file = &object::File::parse(&*map).unwrap();
    if let Ok(uuid) = file.mach_uuid() {
        for candidate in path.parent().unwrap().read_dir().unwrap() {
            let path = candidate.unwrap().path();
            if !path.to_str().unwrap().ends_with(".dSYM") {
                continue;
            }
            for candidate in path.join("Contents/Resources/DWARF").read_dir().unwrap() {
                let path = candidate.unwrap().path();
                let file = File::open(&path).unwrap();
                let map = unsafe { memmap2::Mmap::map(&file).unwrap() };
                let file = &object::File::parse(&*map).unwrap();
                if file.mach_uuid().unwrap() == uuid {
                    return map;
                }
            }
        }
    }

    return map;
}

#[test]
fn correctness() {
    let map = find_debuginfo();
    let file = &object::File::parse(&*map).unwrap();
    let module_base = file.relative_address_base();

    let endian = if file.is_little_endian() {
        gimli::RunTimeEndian::Little
    } else {
        gimli::RunTimeEndian::Big
    };

    fn load_section<'data: 'file, 'file, O, Endian>(
        id: gimli::SectionId,
        file: &'file O,
        endian: Endian,
    ) -> Result<gimli::EndianArcSlice<Endian>, gimli::Error>
    where
        O: object::Object<'data, 'file>,
        Endian: gimli::Endianity,
    {
        use object::ObjectSection;

        let data = file
            .section_by_name(id.name())
            .and_then(|section| section.uncompressed_data().ok())
            .unwrap_or(Cow::Borrowed(&[]));
        Ok(gimli::EndianArcSlice::new(Arc::from(&*data), endian))
    }

    let dwarf = gimli::Dwarf::load(|id| load_section(id, file, endian)).unwrap();
    let ctx = Context::from_dwarf(dwarf).unwrap();
    let mut split_dwarf_loader = addr2line::builtin_split_dwarf_loader::SplitDwarfLoader::new(
        |data, endian| gimli::EndianArcSlice::new(Arc::from(&*data), endian),
        None,
    );

    let mut bias = None;
    TargetSharedLibrary::each(|lib| {
        bias = Some((lib.virtual_memory_bias().0 as u64).wrapping_sub(module_base));
        IterationControl::Break
    });

    #[allow(unused_mut)]
    let mut test = |sym: u64, expected_prefix: &str| {
        let ip = sym.wrapping_sub(bias.unwrap());

        let frames = ctx.find_frames(ip);
        let frames = split_dwarf_loader.run(frames).unwrap();
        let frame = frames.last().unwrap().unwrap();
        let name = frame.function.as_ref().unwrap().demangle().unwrap();
        // Old rust versions generate DWARF with wrong linkage name,
        // so only check the start.
        if !name.starts_with(expected_prefix) {
            panic!("incorrect name '{}', expected {:?}", name, expected_prefix);
        }
    };

    test(test_function as u64, "correctness::test_function");
    test(
        small::test_function as u64,
        "correctness::small::test_function",
    );
    test(auxiliary::foo as u64, "auxiliary::foo");
}

mod small {
    pub fn test_function() {
        println!("y");
    }
}

fn test_function() {
    println!("x");
}

#[test]
fn zero_function() {
    let map = find_debuginfo();
    let file = &object::File::parse(&*map).unwrap();
    let ctx = Context::new(file).unwrap();
    for probe in 0..10 {
        assert!(
            ctx.find_frames(probe)
                .skip_all_loads()
                .unwrap()
                .count()
                .unwrap()
                < 10
        );
    }
}
