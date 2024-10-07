forgetest!(test_eip712, |prj, cmd| {
    let path = prj
        .add_source(
            "Structs",
            r#"
library Structs {
    struct Foo {
        Bar bar;
    }

    struct Bar {
        Art art;
    }

    struct Art {
        uint256 id;
    }

    struct Complex {
        Structs2.Foo foo2;
        Foo[] foos;
    }
}

library Structs2 {
    struct Foo {
        uint256 id;
    }
}
"#,
        )
        .unwrap();

    cmd.forge_fuse().args(["eip712", path.to_string_lossy().as_ref()]).assert_success().stdout_eq(
        str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
No files changed, compilation skipped
Foo(Bar bar)Art(uint256 id)Bar(Art art)

Bar(Art art)Art(uint256 id)

Art(uint256 id)

Complex(Foo foo2,Foo_1[] foos)Art(uint256 id)Bar(Art art)Foo(uint256 id)Foo_1(Bar bar)

Foo(uint256 id)


"#]],
    );
});
