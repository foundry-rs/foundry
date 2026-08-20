// config: bracket_spacing = true
struct Foo {}

struct Bar {
    uint256 foo;
    string bar;
}

struct EmptyStructWithComment { /* body */ }

struct NonEmptyStructTrailingComments {
    uint256 x;
    /* one */ /* two */ }

struct MyStruct {
    // first 1
    // first 2
    uint256 field1;
    // second
    uint256 field2;
}
