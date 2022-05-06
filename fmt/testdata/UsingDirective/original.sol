contract UsingExampleContract {
 using  UsingExampleLibrary      for   *  ;
    using UsingExampleLibrary for uint[];
//    using UsingExampleLibrary for uint[]; // uint -> uint256 is not supported yet
//    using Example.UsingExampleLibrary for uint; // "." in library is not supported yet
}