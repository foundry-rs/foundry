contract ContractDefinition1 is Contract1, Contract2{}

contract ContractDefinition2 is Contract1, Contract2, Contract3Contract3Contract3Contract3Contract3Contract3Contract3, Contract4, Contract5{}

contract ContractDefinition1 is Contract1, Contract2{
    using A for uint;
}

contract ContractDefinition2 is Contract1, Contract2, Contract3Contract3Contract3Contract3Contract3Contract3Contract3, Contract4, Contract5{
    using A for uint;
}
