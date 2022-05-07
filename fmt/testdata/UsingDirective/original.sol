//using UsingExampleLibrary for *; // "using" is not allowed on Source Unit level yet: https://github.com/hyperledger-labs/solang/issues/804

contract UsingExampleContract {
 using  UsingExampleLibrary      for   *  ;
    using UsingExampleLibrary for uint;
//    using Example.UsingExampleLibrary for uint; // "." in library is not supported yet: https://github.com/hyperledger-labs/solang/issues/801
//    using {M.g, M.f} for uint; // "{ ... }" is not supported yet: https://github.com/hyperledger-labs/solang/issues/802
//    using UsingExampleLibrary for uint global; // "global" is not supported yet: https://github.com/hyperledger-labs/solang/issues/803
}