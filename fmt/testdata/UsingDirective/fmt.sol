contract UsingExampleContract {
    using UsingExampleLibrary for *;
    using UsingExampleLibrary for uint256;
    using Example.UsingExampleLibrary for uint256;
    using {M.g, M.f} for uint256;
    using UsingExampleLibrary for uint256 global;
}
