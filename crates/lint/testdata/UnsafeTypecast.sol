/// forge-lint: disable-start(mixed-case-variable)
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

    function downcastUnsafeUint() public pure {
        uint256 a = type(uint256).max;
        uint248 b = uint248(a); //~WARN: typecasts that can truncate values should be checked
        uint240 c = uint240(b); //~WARN: typecasts that can truncate values should be checked
        uint232 d = uint232(c); //~WARN: typecasts that can truncate values should be checked
        uint224 e = uint224(d); //~WARN: typecasts that can truncate values should be checked
        uint216 f = uint216(e); //~WARN: typecasts that can truncate values should be checked
        uint208 g = uint208(f); //~WARN: typecasts that can truncate values should be checked
        uint200 h = uint200(g); //~WARN: typecasts that can truncate values should be checked
        uint192 i = uint192(h); //~WARN: typecasts that can truncate values should be checked
        uint184 j = uint184(i); //~WARN: typecasts that can truncate values should be checked
        uint176 k = uint176(j); //~WARN: typecasts that can truncate values should be checked
        uint168 l = uint168(k); //~WARN: typecasts that can truncate values should be checked
        uint160 m = uint160(l); //~WARN: typecasts that can truncate values should be checked
        uint152 n = uint152(m); //~WARN: typecasts that can truncate values should be checked
        uint144 o = uint144(n); //~WARN: typecasts that can truncate values should be checked
        uint136 p = uint136(o); //~WARN: typecasts that can truncate values should be checked
        uint128 q = uint128(p); //~WARN: typecasts that can truncate values should be checked
        uint120 r = uint120(q); //~WARN: typecasts that can truncate values should be checked
        uint112 s = uint112(r); //~WARN: typecasts that can truncate values should be checked
        uint104 t = uint104(s); //~WARN: typecasts that can truncate values should be checked
        uint96 u = uint96(t);   //~WARN: typecasts that can truncate values should be checked
        uint88 v = uint88(u);   //~WARN: typecasts that can truncate values should be checked
        uint80 w = uint80(v);   //~WARN: typecasts that can truncate values should be checked
        uint72 x = uint72(w);   //~WARN: typecasts that can truncate values should be checked
        uint64 y = uint64(x);   //~WARN: typecasts that can truncate values should be checked
        uint56 z = uint56(y);   //~WARN: typecasts that can truncate values should be checked
        uint48 A = uint48(z);   //~WARN: typecasts that can truncate values should be checked
        uint40 B = uint40(A);   //~WARN: typecasts that can truncate values should be checked
        uint32 C = uint32(B);   //~WARN: typecasts that can truncate values should be checked
        uint24 D = uint24(C);   //~WARN: typecasts that can truncate values should be checked
        uint16 E = uint16(D);   //~WARN: typecasts that can truncate values should be checked
        uint8 F = uint8(E);     //~WARN: typecasts that can truncate values should be checked
    }

    function downcastUnsafeInt() public pure {
        int256 a = type(int256).max;
        int248 b = int248(a); //~WARN: typecasts that can truncate values should be checked
        int240 c = int240(b); //~WARN: typecasts that can truncate values should be checked
        int232 d = int232(c); //~WARN: typecasts that can truncate values should be checked
        int224 e = int224(d); //~WARN: typecasts that can truncate values should be checked
        int216 f = int216(e); //~WARN: typecasts that can truncate values should be checked
        int208 g = int208(f); //~WARN: typecasts that can truncate values should be checked
        int200 h = int200(g); //~WARN: typecasts that can truncate values should be checked
        int192 i = int192(h); //~WARN: typecasts that can truncate values should be checked
        int184 j = int184(i); //~WARN: typecasts that can truncate values should be checked
        int176 k = int176(j); //~WARN: typecasts that can truncate values should be checked
        int168 l = int168(k); //~WARN: typecasts that can truncate values should be checked
        int160 m = int160(l); //~WARN: typecasts that can truncate values should be checked
        int152 n = int152(m); //~WARN: typecasts that can truncate values should be checked
        int144 o = int144(n); //~WARN: typecasts that can truncate values should be checked
        int136 p = int136(o); //~WARN: typecasts that can truncate values should be checked
        int128 q = int128(p); //~WARN: typecasts that can truncate values should be checked
        int120 r = int120(q); //~WARN: typecasts that can truncate values should be checked
        int112 s = int112(r); //~WARN: typecasts that can truncate values should be checked
        int104 t = int104(s); //~WARN: typecasts that can truncate values should be checked
        int96 u = int96(t);   //~WARN: typecasts that can truncate values should be checked
        int88 v = int88(u);   //~WARN: typecasts that can truncate values should be checked
        int80 w = int80(v);   //~WARN: typecasts that can truncate values should be checked
        int72 x = int72(w);   //~WARN: typecasts that can truncate values should be checked
        int64 y = int64(x);   //~WARN: typecasts that can truncate values should be checked
        int56 z = int56(y);   //~WARN: typecasts that can truncate values should be checked
        int48 A = int48(z);   //~WARN: typecasts that can truncate values should be checked
        int40 B = int40(A);   //~WARN: typecasts that can truncate values should be checked
        int32 C = int32(B);   //~WARN: typecasts that can truncate values should be checked
        int24 D = int24(C);   //~WARN: typecasts that can truncate values should be checked
        int16 E = int16(D);   //~WARN: typecasts that can truncate values should be checked
        int8 F = int8(E);     //~WARN: typecasts that can truncate values should be checked
    }

    function upcastSafeBytes() public pure {
        bytes1 a = 0x12;
        bytes2 b = bytes2(a);
        bytes4 c = bytes4(b);
        bytes8 d = bytes8(c);
        bytes16 e = bytes16(d);
        bytes24 f = bytes24(e);
        bytes28 g = bytes28(f);
        bytes30 h = bytes30(g);
        bytes31 i = bytes31(h);
        bytes32 j = bytes32(i);

        // Safe: same size copies
        bytes1 aa = bytes1(a);
        bytes2 bb = bytes2(b);
        bytes4 cc = bytes4(c);
        bytes8 dd = bytes8(d);
        bytes16 ee = bytes16(e);
        bytes24 ff = bytes24(f);
        bytes28 gg = bytes28(g);
        bytes30 hh = bytes30(h);
        bytes31 ii = bytes31(i);
        bytes32 jj = bytes32(j);
    }

    function numericDowncastsUnsafe() public pure {
        uint256 a = 2**255 + 1000;

        uint248 a_u248 = uint248(a); //~WARN: typecasts that can truncate values should be checked
        uint240 a_u240 = uint240(a); //~WARN: typecasts that can truncate values should be checked
        uint224 a_u224 = uint224(a); //~WARN: typecasts that can truncate values should be checked
        uint192 a_u192 = uint192(a); //~WARN: typecasts that can truncate values should be checked
        uint160 a_u160 = uint160(a); //~WARN: typecasts that can truncate values should be checked
        uint128 a_u128 = uint128(a); //~WARN: typecasts that can truncate values should be checked
        uint64 a_u64 = uint64(a);    //~WARN: typecasts that can truncate values should be checked
        uint32 a_u32 = uint32(a);    //~WARN: typecasts that can truncate values should be checked
        uint16 a_u16 = uint16(a);    //~WARN: typecasts that can truncate values should be checked
        uint8 a_u8 = uint8(a);       //~WARN: typecasts that can truncate values should be checked

        int256 i = -2**255 + 1000;
        int248 i_i248 = int248(i); //~WARN: typecasts that can truncate values should be checked
        int240 i_i240 = int240(i); //~WARN: typecasts that can truncate values should be checked
        int224 i_i224 = int224(i); //~WARN: typecasts that can truncate values should be checked
        int192 i_i192 = int192(i); //~WARN: typecasts that can truncate values should be checked
        int160 i_i160 = int160(i); //~WARN: typecasts that can truncate values should be checked
        int128 i_i128 = int128(i); //~WARN: typecasts that can truncate values should be checked
        int64 i_i64 = int64(i);    //~WARN: typecasts that can truncate values should be checked
        int32 i_i32 = int32(i);    //~WARN: typecasts that can truncate values should be checked
        int16 i_i16 = int16(i);    //~WARN: typecasts that can truncate values should be checked
        int8 i_i8 = int8(i);       //~WARN: typecasts that can truncate values should be checked
    }

    function numericUpcastsSafe() public pure {
        int256 i = -2**255 + 1000;
        uint256 a = 2**255 + 1000;

        uint128 s_u128 = 1234;
        uint256 s_u256 = uint256(s_u128); // Safe

        int128 s_i128 = -1234;
        int256 s_i256 = int256(s_i128); // Safe

        uint256 s_u256_copy = uint256(a);
        int256 s_i256_copy = int256(i);
    }

    function downcastUnsafeBytes() public pure {
        bytes32 a = "hello world";

        bytes31 b = bytes31(a); //~WARN: typecasts that can truncate values should be checked
        bytes30 c = bytes30(b); //~WARN: typecasts that can truncate values should be checked
        bytes28 d = bytes28(c); //~WARN: typecasts that can truncate values should be checked
        bytes24 e = bytes24(d); //~WARN: typecasts that can truncate values should be checked
        bytes20 f = bytes20(e); //~WARN: typecasts that can truncate values should be checked
        bytes16 g = bytes16(f); //~WARN: typecasts that can truncate values should be checked
        bytes12 h = bytes12(g); //~WARN: typecasts that can truncate values should be checked
        bytes8 i = bytes8(h);   //~WARN: typecasts that can truncate values should be checked
        bytes4 j = bytes4(i);   //~WARN: typecasts that can truncate values should be checked
        bytes2 k = bytes2(j);   //~WARN: typecasts that can truncate values should be checked
        bytes1 l = bytes1(k);   //~WARN: typecasts that can truncate values should be checked

        bytes memory dyn = "hello world";
        bytes32 fromDyn = bytes32(dyn); //~WARN: typecasts that can truncate values should be checked

        string memory s = "hello world";
        bytes32 fromStr = bytes32(bytes(s)); //~WARN: typecasts that can truncate values should be checked
    }

    function signedUnsignedConversions() public pure {
        int256 a = -1;
        uint256 b = uint256(a); //~WARN: typecasts that can truncate values should be checked

        int128 a128 = -500;
        uint128 b128 = uint128(a128); //~WARN: typecasts that can truncate values should be checked

        uint256 c = type(uint256).max;
        int256 d = int256(c); //~WARN: typecasts that can truncate values should be checked

        uint128 c128 = type(uint128).max;
        int128 d128 = int128(c128); //~WARN: typecasts that can truncate values should be checked

        int128 safePos = 1234;
        uint128 u_safe = uint128(safePos); //~WARN: typecasts that can truncate values should be checked

        uint8 small = 5;
        int8 signedSmall = int8(small); //~WARN: typecasts that can truncate values should be checked
    }

    function upcastSafeAddress() public pure {
        address a = 0x1234567890123456789012345678901234567890;

        uint160 b = uint160(a); // Safe
        uint176 c = uint176(b); // Safe
        uint192 d = uint192(c); // Safe
        uint224 e = uint224(d); // Safe
        uint256 f = uint256(e); // Safe
    }

    function downcastUnsafeAddress() public pure {
        address a = 0x1234567890123456789012345678901234567890;
        uint160 base = uint160(a);

        uint152 b = uint152(base); //~WARN: typecasts that can truncate values should be checked
        uint144 c = uint144(base); //~WARN: typecasts that can truncate values should be checked
        uint136 d = uint136(base); //~WARN: typecasts that can truncate values should be checked
        uint128 e = uint128(base); //~WARN: typecasts that can truncate values should be checked
        uint120 f = uint120(base); //~WARN: typecasts that can truncate values should be checked
        uint112 g = uint112(base); //~WARN: typecasts that can truncate values should be checked
        uint104 h = uint104(base); //~WARN: typecasts that can truncate values should be checked
        uint96 i = uint96(base);   //~WARN: typecasts that can truncate values should be checked
        uint88 j = uint88(base);   //~WARN: typecasts that can truncate values should be checked
    }

    function downcastUnsafeAddress2() public pure {
        address a = 0x1234567890123456789012345678901234567890;
        uint160 base = uint160(a);

        uint80 k = uint80(base);   //~WARN: typecasts that can truncate values should be checked
        uint72 l = uint72(base);   //~WARN: typecasts that can truncate values should be checked
        uint64 m = uint64(base);   //~WARN: typecasts that can truncate values should be checked
        uint56 n = uint56(base);   //~WARN: typecasts that can truncate values should be checked
        uint48 o = uint48(base);   //~WARN: typecasts that can truncate values should be checked
        uint40 p = uint40(base);   //~WARN: typecasts that can truncate values should be checked
        uint32 q = uint32(base);   //~WARN: typecasts that can truncate values should be checked
        uint24 r = uint24(base);   //~WARN: typecasts that can truncate values should be checked
        uint16 s = uint16(base);   //~WARN: typecasts that can truncate values should be checked
        uint8 t = uint8(base);     //~WARN: typecasts that can truncate values should be checked
    }

    function repro() public pure {
        bytes memory stringToBytes = bytes("Initializable: contract is already initialized");
    }
}
/// forge-lint: disable-end(mixed-case-variable)
