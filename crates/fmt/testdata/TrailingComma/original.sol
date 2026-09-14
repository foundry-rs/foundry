import {A, B,} from "a.sol";

contract C is Contract {
    enum E { A, B, }
    event Event(uint256 a,);
    error Error(uint256 a,);
    using {f, f2,} for uint256;

    constructor(uint256 a,) {}
    function f(uint256 a, ) external {}
    function f2(uint256 a, bytes32 b,) external returns (uint256,) {}

    function f3() external {
        try some.invoke() returns (uint256,uint256,) {} catch {}
    }

    function f4() external {
        f(1,);
        f({a: 1,});
        this.f{gas: 100,}(1,);
        uint256[2] memory values = [uint256(1), 2,];
        (uint256 a,) = f2(1, bytes32(0),);
        (,a,) = some.invoke();
        emit Event(1,);
        revert Error(1,);
    }
}
