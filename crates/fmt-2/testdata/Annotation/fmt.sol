// Support for Solana/Substrate annotations
contract A {
    @selector([1, 2, 3, 4])
    function foo() public {}

    @selector("another one")
    function bar() public {}

    @first("")
    @second("")
    function foobar() public {}
}

@topselector(2)
contract B {}
