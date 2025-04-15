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
        Rec[][] recs;
    }

    struct Rec {
        Rec[] rec;
    }
}

library Structs2 {
    struct Foo {
        uint256 id;
    }

    struct Rec {
        Bar[] bar;
    }

    struct Bar {
        Rec rec;
    }

    struct FooBar {
        Foo[] foos;
        Bar[] bars;
        Structs.Foo foo;
        Structs.Bar bar;
        Rec[] recs;
        Structs.Rec rec;
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

Complex(Foo foo2,Foo_1[] foos,Rec[][] recs)Art(uint256 id)Bar(Art art)Foo(uint256 id)Foo_1(Bar bar)Rec(Rec[] rec)

Rec(Rec[] rec)

Foo(uint256 id)

Rec(Bar[] bar)Bar(Rec rec)

Bar(Rec rec)Rec(Bar[] bar)

FooBar(Foo[] foos,Bar[] bars,Foo_1 foo,Bar_1 bar,Rec[] recs,Rec_1 rec)Art(uint256 id)Bar(Rec rec)Bar_1(Art art)Foo(uint256 id)Foo_1(Bar_1 bar)Rec(Bar[] bar)Rec_1(Rec_1[] rec)


"#]],
    );
});
