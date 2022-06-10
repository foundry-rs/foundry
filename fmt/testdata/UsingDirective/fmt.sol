contract UsingExampleContract {
    using UsingExampleLibrary for *;
    using UsingExampleLibrary for uint256;
    using Example.UsingExampleLibrary for uint256;
    using {M.g, M.f} for uint256;
    using UsingExampleLibrary for uint256 global;
    using {
        These,
        Are,
        MultipleLibraries,
        ThatNeedToBePut,
        OnSeparateLines
    } for uint256;
    using {
        This
            .isareally
            .longmember
            .access
            .expression
            .that
            .needs
            .to
            .besplit
            .into
            .lines
    } for uint256;
}
