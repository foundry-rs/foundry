
library L {
    function sq(uint256 x) internal pure returns (uint256) {
        return x * x;
    }
}

contract Base {
    function v(uint256 x) internal virtual returns (uint256) {
        return x + 10;
    }
}

contract C is Base {
    uint256 public s;
    function(uint256) internal pure returns (uint256) fp;

    modifier onlyPositive(uint256 x) {
        require(x > 0, "neg");
        _;
    }

    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }

    function fact(uint256 n) internal pure returns (uint256) {
        return n == 0 ? 1 : n * fact(n - 1);
    }

    function tailInner(uint256 x) internal returns (uint256) {
        s = x;
        return x * 3;
    }

    function tailOuter(uint256 x) internal returns (uint256) {
        return tailInner(x + 1);
    }

    function tiny(uint256 x) internal pure returns (uint256) {
        return x + 1;
    }

    function dbl(uint256 x) internal pure returns (uint256) {
        return x * 2;
    }

    function trp(uint256 x) internal pure returns (uint256) {
        return x * 3;
    }

    function runAdd(uint256 a, uint256 b) external pure returns (uint256) {
        return add(a, b);
    }

    function runFact(uint256 n) external pure returns (uint256) {
        return fact(n);
    }

    function runTail(uint256 x) external returns (uint256) {
        return tailOuter(x);
    }

    function runTiny(uint256 x) external pure returns (uint256) {
        return tiny(x) + tiny(x + 5);
    }

    function runMod(uint256 x) external onlyPositive(x) returns (uint256) {
        s = x;
        return x;
    }

    function runPtr(uint256 x, bool d) external returns (uint256) {
        fp = d ? dbl : trp;
        return fp(x);
    }

    function v(uint256 x) internal override returns (uint256) {
        return super.v(x) * 2;
    }

    function asmFn(uint256 x) internal pure returns (uint256 r) {
        assembly {
            function twice(a) -> b {
                b := add(a, a)
            }
            r := twice(x)
        }
    }

    function asmSub(uint256 x) internal pure returns (uint256 r) {
        assembly {
            function sub2(a, b) -> c {
                c := sub(a, b)
            }
            r := sub2(x, 3)
        }
    }

    function runAsmSub(uint256 x) external pure returns (uint256) {
        return asmSub(x);
    }

    function runLib(uint256 x) external pure returns (uint256) {
        return L.sq(x);
    }

    function runSuper(uint256 x) external returns (uint256) {
        return v(x);
    }

    function runAsm(uint256 x) external pure returns (uint256) {
        return asmFn(x);
    }
}

contract CTest {
    function test_asm_sub() public {
        new C().runAsmSub(10);
    }

    function test_add() public {
        new C().runAdd(3, 4);
    }

    function test_fact() public {
        new C().runFact(3);
    }

    function test_tail() public {
        new C().runTail(5);
    }

    function test_tiny() public {
        new C().runTiny(7);
    }

    function test_mod() public {
        new C().runMod(1);
    }

    function test_ptr() public {
        new C().runPtr(5, true);
    }

    function test_lib() public {
        new C().runLib(3);
    }

    function test_super() public {
        new C().runSuper(3);
    }

    function test_asm() public {
        new C().runAsm(3);
    }
}
