contract UnsafeTypecast {
    function upcastSafeUint() public pure {
        uint8 a = type(uint8).max;
        uint16 b = uint16(a);
        uint24 c = uint24(b);
        uint32 d = uint32(c);
        uint40 e = uint40(d);
        uint48 f = uint48(e);
        uint56 g = uint56(f);
        uint64 h = uint64(g);
        uint72 i = uint72(h);
        uint80 j = uint80(i);
        uint88 k = uint88(j);
        uint96 l = uint96(k);
        uint104 m = uint104(l);
        uint112 n = uint112(m);
        uint120 o = uint120(n);
        uint128 p = uint128(o);
        uint136 q = uint136(p);
        uint144 r = uint144(q);
        uint152 s = uint152(r);
        uint160 t = uint160(s);
        uint168 u = uint168(t);
        uint176 v = uint176(u);
        uint184 w = uint184(v);
        uint192 x = uint192(w);
        uint200 y = uint200(x);
        uint208 z = uint208(y);
        uint216 A = uint216(z);
        uint224 B = uint224(A);
        uint232 C = uint232(B);
        uint240 D = uint240(C);
        uint248 E = uint248(D);
        uint256 F = uint256(E);
    }
    function upcastSafeInt() public pure {
        int8 a = type(int8).max;
        int16 b = int16(a);
        int24 c = int24(b);
        int32 d = int32(c);
        int40 e = int40(d);
        int48 f = int48(e);
        int56 g = int56(f);
        int64 h = int64(g);
        int72 i = int72(h);
        int80 j = int80(i);
        int88 k = int88(j);
        int96 l = int96(k);
        int104 m = int104(l);
        int112 n = int112(m);
        int120 o = int120(n);
        int128 p = int128(o);
        int136 q = int136(p);
        int144 r = int144(q);
        int152 s = int152(r);
        int160 t = int160(s);
        int168 u = int168(t);
        int176 v = int176(u);
        int184 w = int184(v);
        int192 x = int192(w);
        int200 y = int200(x);
        int208 z = int208(y);
        int216 A = int216(z);
        int224 B = int224(A);
        int232 C = int232(B);
        int240 D = int240(C);
        int248 E = int248(D);
        int256 F = int256(E);
    }
    function downcastSafeUint() public pure {
        uint256 a = type(uint256).max;
        uint248 b = uint248(a); //~WARN: typecasts that can truncate values should be avoided
        uint240 c = uint240(b); //~WARN: typecasts that can truncate values should be avoided
        uint232 d = uint232(c); //~WARN: typecasts that can truncate values should be avoided
        uint224 e = uint224(d); //~WARN: typecasts that can truncate values should be avoided
        uint216 f = uint216(e); //~WARN: typecasts that can truncate values should be avoided
        uint208 g = uint208(f); //~WARN: typecasts that can truncate values should be avoided
        uint200 h = uint200(g); //~WARN: typecasts that can truncate values should be avoided
        uint192 i = uint192(h); //~WARN: typecasts that can truncate values should be avoided
        uint184 j = uint184(i); //~WARN: typecasts that can truncate values should be avoided
        uint176 k = uint176(j); //~WARN: typecasts that can truncate values should be avoided
        uint168 l = uint168(k); //~WARN: typecasts that can truncate values should be avoided
        uint160 m = uint160(l); //~WARN: typecasts that can truncate values should be avoided
        uint152 n = uint152(m); //~WARN: typecasts that can truncate values should be avoided
        uint144 o = uint144(n); //~WARN: typecasts that can truncate values should be avoided
        uint136 p = uint136(o); //~WARN: typecasts that can truncate values should be avoided
        uint128 q = uint128(p); //~WARN: typecasts that can truncate values should be avoided
        uint120 r = uint120(q); //~WARN: typecasts that can truncate values should be avoided
        uint112 s = uint112(r); //~WARN: typecasts that can truncate values should be avoided
        uint104 t = uint104(s); //~WARN: typecasts that can truncate values should be avoided
        uint96 u = uint96(t); //~WARN: typecasts that can truncate values should be avoided
        uint88 v = uint88(u); //~WARN: typecasts that can truncate values should be avoided
        uint80 w = uint80(v); //~WARN: typecasts that can truncate values should be avoided
        uint72 x = uint72(w); //~WARN: typecasts that can truncate values should be avoided
        uint64 y = uint64(x); //~WARN: typecasts that can truncate values should be avoided
        uint56 z = uint56(y); //~WARN: typecasts that can truncate values should be avoided
        uint48 A = uint48(z); //~WARN: typecasts that can truncate values should be avoided
        uint40 B = uint40(A); //~WARN: typecasts that can truncate values should be avoided
        uint32 C = uint32(B); //~WARN: typecasts that can truncate values should be avoided
        uint24 D = uint24(C); //~WARN: typecasts that can truncate values should be avoided
        uint16 E = uint16(D); //~WARN: typecasts that can truncate values should be avoided
        uint8 F = uint8(E); //~WARN: typecasts that can truncate values should be avoided
    }
    function downcastSafeInt() public pure {
        int256 a = type(int256).max;
        int248 b = int248(a); //~WARN: typecasts that can truncate values should be avoided
        int240 c = int240(b); //~WARN: typecasts that can truncate values should be avoided
        int232 d = int232(c); //~WARN: typecasts that can truncate values should be avoided
        int224 e = int224(d); //~WARN: typecasts that can truncate values should be avoided
        int216 f = int216(e); //~WARN: typecasts that can truncate values should be avoided
        int208 g = int208(f); //~WARN: typecasts that can truncate values should be avoided
        int200 h = int200(g); //~WARN: typecasts that can truncate values should be avoided
        int192 i = int192(h); //~WARN: typecasts that can truncate values should be avoided
        int184 j = int184(i); //~WARN: typecasts that can truncate values should be avoided
        int176 k = int176(j); //~WARN: typecasts that can truncate values should be avoided
        int168 l = int168(k); //~WARN: typecasts that can truncate values should be avoided
        int160 m = int160(l); //~WARN: typecasts that can truncate values should be avoided
        int152 n = int152(m); //~WARN: typecasts that can truncate values should be avoided
        int144 o = int144(n); //~WARN: typecasts that can truncate values should be avoided
        int136 p = int136(o); //~WARN: typecasts that can truncate values should be avoided
        int128 q = int128(p); //~WARN: typecasts that can truncate values should be avoided
        int120 r = int120(q); //~WARN: typecasts that can truncate values should be avoided
        int112 s = int112(r); //~WARN: typecasts that can truncate values should be avoided
        int104 t = int104(s); //~WARN: typecasts that can truncate values should be avoided
        int96 u = int96(t); //~WARN: typecasts that can truncate values should be avoided
        int88 v = int88(u); //~WARN: typecasts that can truncate values should be avoided
        int80 w = int80(v); //~WARN: typecasts that can truncate values should be avoided
        int72 x = int72(w); //~WARN: typecasts that can truncate values should be avoided
        int64 y = int64(x); //~WARN: typecasts that can truncate values should be avoided
        int56 z = int56(y); //~WARN: typecasts that can truncate values should be avoided
        int48 A = int48(z); //~WARN: typecasts that can truncate values should be avoided
        int40 B = int40(A); //~WARN: typecasts that can truncate values should be avoided
        int32 C = int32(B); //~WARN: typecasts that can truncate values should be avoided
        int24 D = int24(C); //~WARN: typecasts that can truncate values should be avoided
        int16 E = int16(D); //~WARN: typecasts that can truncate values should be avoided
        int8 F = int8(E); //~WARN: typecasts that can truncate values should be avoided
    }
    function numericDowncasts() public {
        // Unsafe: uint256 -> uint128
        uint256 a = 1000;
        uint128 b = uint128(a); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: uint256 -> uint64
        uint64 c = uint64(a); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: uint256 -> uint8
        uint8 d = uint8(a); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: int256 -> int128
        int256 e = -1000;
        int128 f = int128(e); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: int256 -> int8
        int8 g = int8(e); //~WARN: typecasts that can truncate values should be avoided
        
        // Safe: uint128 -> uint256 (upcast)
        uint256 h = uint256(b);
        
        // Safe: int128 -> int256 (upcast)
        int256 i = int256(f);
        
        // Safe: same size
        uint256 j = uint256(a);
    }
    
    function signedUnsignedConversions() public {
        // Unsafe: int256 -> uint256 (potential loss of sign)
        int256 a = -1000;
        uint256 b = uint256(a); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: int128 -> uint256 (potential loss of sign)
        int128 c = -100;
        uint256 d = uint256(c); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: uint256 -> int256 (potential overflow)
        uint256 e = 1000;
        int256 f = int256(e); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: uint256 -> int128 (potential overflow and truncation)
        int128 g = int128(e); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: uint128 -> int128 (potential overflow)
        uint128 h = 100;
        int128 i = int128(h); //~WARN: typecasts that can truncate values should be avoided
    }
    
    function bytesConversions() public {
        // Unsafe: bytes32 -> bytes16 (truncation)
        bytes32 a = "hello world";
        bytes16 b = bytes16(a); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: bytes32 -> bytes8 (truncation)
        bytes8 c = bytes8(a); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: bytes -> bytes32 (potential truncation)
        bytes memory d = "hello world";
        bytes32 e = bytes32(d); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: string -> bytes32 (potential truncation)
        string memory f = "hello world";
        bytes32 g = bytes32(bytes(f)); //~WARN: typecasts that can truncate values should be avoided
        
        // Safe: bytes16 -> bytes32 (upcast)
        bytes32 h = bytes32(b);
        
        // Safe: same size
        bytes32 i = bytes32(a);
    }
    
    function addressConversions() public {
        // Unsafe: address -> uint128 (truncation)
        address a = 0x1234567890123456789012345678901234567890;
        uint128 b = uint128(uint160(a)); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: address -> int256 (sign issues)
        int256 c = int256(uint160(a)); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: address -> uint8 (severe truncation)
        uint8 d = uint8(uint160(a)); //~WARN: typecasts that can truncate values should be avoided
        
        // Safe: address -> uint160 (exact fit)
        uint160 e = uint160(a);
        
        // Safe: address -> uint256 (upcast)
        uint256 f = uint256(uint160(a));
    }
    
    function literalCasts() public {
        // Unsafe: literal downcasts
        uint128 a = uint128(1000000000000000000000000000000000000000); //~WARN: typecasts that can truncate values should be avoided
        uint64 b = uint64(1000000000000000000000); //~WARN: typecasts that can truncate values should be avoided
        int128 c = int128(-1000000000000000000000000000000000000000); //~WARN: typecasts that can truncate values should be avoided
        
        // Unsafe: signed/unsigned conversions
        uint256 d = uint256(-1000); //~WARN: typecasts that can truncate values should be avoided
        int256 e = int256(1000000000000000000000000000000000000000); //~WARN: typecasts that can truncate values should be avoided
        
        // Safe: literal upcasts
        uint256 f = uint256(1000);
        int256 g = int256(-1000);
    }
} 