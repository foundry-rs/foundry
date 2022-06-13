contract UsingExampleContract {
 using  UsingExampleLibrary      for   *  ;
    using UsingExampleLibrary for uint;
   using Example.UsingExampleLibrary  for  uint;
        using { M.g, M.f} for uint;
using UsingExampleLibrary for   uint  global;
using { These, Are, MultipleLibraries, ThatNeedToBePut, OnSeparateLines } for uint;
using { This.isareally.longmember.access.expression.that.needs.to.besplit.into.lines } for uint;
}
