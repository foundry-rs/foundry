// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// forge-lint: disable-start(mixed-case-variable)
contract UnsafeTypecast {
    // Unsigned upcasts are always safe.
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

    // Signed upcasts are safe.
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

    function upcastSafeBytes() public pure {
        bytes1 a = 0xFF;
        bytes2 b = bytes2(a);
        bytes3 c = bytes3(b);
        bytes4 d = bytes4(c);
        bytes5 e = bytes5(d);
        bytes6 f = bytes6(e);
        bytes7 g = bytes7(f);
        bytes8 h = bytes8(g);
        bytes9 i = bytes9(h);
        bytes10 j = bytes10(i);
        bytes11 k = bytes11(j);
        bytes12 l = bytes12(k);
        bytes13 m = bytes13(l);
        bytes14 n = bytes14(m);
        bytes15 o = bytes15(n);
        bytes16 p = bytes16(o);
        bytes17 q = bytes17(p);
        bytes18 r = bytes18(q);
        bytes19 s = bytes19(r);
        bytes20 t = bytes20(s);
        bytes21 u = bytes21(t);
        bytes22 v = bytes22(u);
        bytes23 w = bytes23(v);
        bytes24 x = bytes24(w);
        bytes25 y = bytes25(x);
        bytes26 z = bytes26(y);
        bytes27 A = bytes27(z);
        bytes28 B = bytes28(A);
        bytes29 C = bytes29(B);
        bytes30 D = bytes30(C);
        bytes31 E = bytes31(D);
        bytes32 F = bytes32(E);
    }

    function safeSizeUint() public pure {
        uint256(type(uint256).max);
        uint248(type(uint248).max);
        uint240(type(uint240).max);
        uint232(type(uint232).max);
        uint224(type(uint224).max);
        uint216(type(uint216).max);
        uint208(type(uint208).max);
        uint200(type(uint200).max);
        uint192(type(uint192).max);
        uint184(type(uint184).max);
        uint176(type(uint176).max);
        uint168(type(uint168).max);
        uint160(type(uint160).max);
        uint152(type(uint152).max);
        uint144(type(uint144).max);
        uint136(type(uint136).max);
        uint128(type(uint128).max);
        uint120(type(uint120).max);
        uint112(type(uint112).max);
        uint104(type(uint104).max);
        uint96(type(uint96).max);
        uint88(type(uint88).max);
        uint80(type(uint80).max);
        uint72(type(uint72).max);
        uint64(type(uint64).max);
        uint56(type(uint56).max);
        uint48(type(uint48).max);
        uint40(type(uint40).max);
        uint32(type(uint32).max);
        uint24(type(uint24).max);
        uint16(type(uint16).max);
        uint8(type(uint8).max);
    }

    function safeSizeInt() public pure {
        int256(type(int256).max);
        int248(type(int248).max);
        int240(type(int240).max);
        int232(type(int232).max);
        int224(type(int224).max);
        int216(type(int216).max);
        int208(type(int208).max);
        int200(type(int200).max);
        int192(type(int192).max);
        int184(type(int184).max);
        int176(type(int176).max);
        int168(type(int168).max);
        int160(type(int160).max);
        int152(type(int152).max);
        int144(type(int144).max);
        int136(type(int136).max);
        int128(type(int128).max);
        int120(type(int120).max);
        int112(type(int112).max);
        int104(type(int104).max);
        int96(type(int96).max);
        int88(type(int88).max);
        int80(type(int80).max);
        int72(type(int72).max);
        int64(type(int64).max);
        int56(type(int56).max);
        int48(type(int48).max);
        int40(type(int40).max);
        int32(type(int32).max);
        int24(type(int24).max);
        int16(type(int16).max);
        int8(type(int8).max);
    }

    function sameSizeAddressSafe() public pure {
        address a = 0x1234567890123456789012345678901234567890;
        uint160 b = uint160(a);
        bytes20 c = bytes20(a);
        address d = address(a);
        // The following tests, `downcastUnsafeUint` and `downcastUnsafeBytes`, verify that other downcasts
        // would also throw. Additionally, the compiler prevents direct casting of addresses to smaller types.
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
        uint96 u = uint96(t); //~WARN: typecasts that can truncate values should be checked
        uint88 v = uint88(u); //~WARN: typecasts that can truncate values should be checked
        uint80 w = uint80(v); //~WARN: typecasts that can truncate values should be checked
        uint72 x = uint72(w); //~WARN: typecasts that can truncate values should be checked
        uint64 y = uint64(x); //~WARN: typecasts that can truncate values should be checked
        uint56 z = uint56(y); //~WARN: typecasts that can truncate values should be checked
        uint48 A = uint48(z); //~WARN: typecasts that can truncate values should be checked
        uint40 B = uint40(A); //~WARN: typecasts that can truncate values should be checked
        uint32 C = uint32(B); //~WARN: typecasts that can truncate values should be checked
        uint24 D = uint24(C); //~WARN: typecasts that can truncate values should be checked
        uint16 E = uint16(D); //~WARN: typecasts that can truncate values should be checked
        uint8 F = uint8(E); //~WARN: typecasts that can truncate values should be checked
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
        int96 u = int96(t); //~WARN: typecasts that can truncate values should be checked
        int88 v = int88(u); //~WARN: typecasts that can truncate values should be checked
        int80 w = int80(v); //~WARN: typecasts that can truncate values should be checked
        int72 x = int72(w); //~WARN: typecasts that can truncate values should be checked
        int64 y = int64(x); //~WARN: typecasts that can truncate values should be checked
        int56 z = int56(y); //~WARN: typecasts that can truncate values should be checked
        int48 A = int48(z); //~WARN: typecasts that can truncate values should be checked
        int40 B = int40(A); //~WARN: typecasts that can truncate values should be checked
        int32 C = int32(B); //~WARN: typecasts that can truncate values should be checked
        int24 D = int24(C); //~WARN: typecasts that can truncate values should be checked
        int16 E = int16(D); //~WARN: typecasts that can truncate values should be checked
        int8 F = int8(E); //~WARN: typecasts that can truncate values should be checked
    }

    function downcastUnsafeBytes() public pure {
        bytes32 a = bytes32(type(uint256).max);
        bytes31 b = bytes31(a); //~WARN: typecasts that can truncate values should be checked
        bytes30 c = bytes30(b); //~WARN: typecasts that can truncate values should be checked
        bytes29 d = bytes29(c); //~WARN: typecasts that can truncate values should be checked
        bytes28 e = bytes28(d); //~WARN: typecasts that can truncate values should be checked
        bytes27 f = bytes27(e); //~WARN: typecasts that can truncate values should be checked
        bytes26 g = bytes26(f); //~WARN: typecasts that can truncate values should be checked
        bytes25 h = bytes25(g); //~WARN: typecasts that can truncate values should be checked
        bytes24 i = bytes24(h); //~WARN: typecasts that can truncate values should be checked
        bytes23 j = bytes23(i); //~WARN: typecasts that can truncate values should be checked
        bytes22 k = bytes22(j); //~WARN: typecasts that can truncate values should be checked
        bytes21 l = bytes21(k); //~WARN: typecasts that can truncate values should be checked
        bytes20 m = bytes20(l); //~WARN: typecasts that can truncate values should be checked
        bytes19 n = bytes19(m); //~WARN: typecasts that can truncate values should be checked
        bytes18 o = bytes18(n); //~WARN: typecasts that can truncate values should be checked
        bytes17 p = bytes17(o); //~WARN: typecasts that can truncate values should be checked
        bytes16 q = bytes16(p); //~WARN: typecasts that can truncate values should be checked
        bytes15 r = bytes15(q); //~WARN: typecasts that can truncate values should be checked
        bytes14 s = bytes14(r); //~WARN: typecasts that can truncate values should be checked
        bytes13 t = bytes13(s); //~WARN: typecasts that can truncate values should be checked
        bytes12 u = bytes12(t); //~WARN: typecasts that can truncate values should be checked
        bytes11 v = bytes11(u); //~WARN: typecasts that can truncate values should be checked
        bytes10 w = bytes10(v); //~WARN: typecasts that can truncate values should be checked
        bytes9 x = bytes9(w); //~WARN: typecasts that can truncate values should be checked
        bytes8 y = bytes8(x); //~WARN: typecasts that can truncate values should be checked
        bytes7 z = bytes7(y); //~WARN: typecasts that can truncate values should be checked
        bytes6 A = bytes6(z); //~WARN: typecasts that can truncate values should be checked
        bytes5 B = bytes5(A); //~WARN: typecasts that can truncate values should be checked
        bytes4 C = bytes4(B); //~WARN: typecasts that can truncate values should be checked
        bytes3 D = bytes3(C); //~WARN: typecasts that can truncate values should be checked
        bytes2 E = bytes2(D); //~WARN: typecasts that can truncate values should be checked
        bytes1 F = bytes1(E); //~WARN: typecasts that can truncate values should be checked
    }

    function unsignedSignedUnsafe() public pure {
        uint256 a = type(uint256).max;
        int256 b = int256(a); //~WARN: typecasts that can truncate values should be checked
        uint248 c = type(uint248).max;
        int248 d = int248(c); //~WARN: typecasts that can truncate values should be checked
        uint240 e = type(uint240).max;
        int240 f = int240(e); //~WARN: typecasts that can truncate values should be checked
        uint232 g = type(uint232).max;
        int232 h = int232(g); //~WARN: typecasts that can truncate values should be checked
        uint224 i = type(uint224).max;
        int224 j = int224(i); //~WARN: typecasts that can truncate values should be checked
        uint216 k = type(uint216).max;
        int216 l = int216(k); //~WARN: typecasts that can truncate values should be checked
        uint208 m = type(uint208).max;
        int208 n = int208(m); //~WARN: typecasts that can truncate values should be checked
        uint200 o = type(uint200).max;
        int200 p = int200(o); //~WARN: typecasts that can truncate values should be checked
        uint192 q = type(uint192).max;
        int192 r = int192(q); //~WARN: typecasts that can truncate values should be checked
        uint184 s = type(uint184).max;
        int184 t = int184(s); //~WARN: typecasts that can truncate values should be checked
        uint176 u = type(uint176).max;
        int176 v = int176(u); //~WARN: typecasts that can truncate values should be checked
        uint168 w = type(uint168).max;
        int168 x = int168(w); //~WARN: typecasts that can truncate values should be checked
        uint160 y = type(uint160).max;
        int160 z = int160(y); //~WARN: typecasts that can truncate values should be checked
        uint152 A = type(uint152).max;
        int152 B = int152(A); //~WARN: typecasts that can truncate values should be checked
        uint144 C = type(uint144).max;
        int144 D = int144(C); //~WARN: typecasts that can truncate values should be checked
        uint136 E = type(uint136).max;
        int136 F = int136(E); //~WARN: typecasts that can truncate values should be checked
        uint128 G = type(uint128).max;
        int128 H = int128(G); //~WARN: typecasts that can truncate values should be checked
        uint120 I = type(uint120).max;
        int120 J = int120(I); //~WARN: typecasts that can truncate values should be checked
        uint112 K = type(uint112).max;
        int112 L = int112(K); //~WARN: typecasts that can truncate values should be checked
        uint104 M = type(uint104).max;
        int104 N = int104(M); //~WARN: typecasts that can truncate values should be checked
        uint96 O = type(uint96).max;
        int96 P = int96(O); //~WARN: typecasts that can truncate values should be checked
        uint88 Q = type(uint88).max;
        int88 R = int88(Q); //~WARN: typecasts that can truncate values should be checked
        uint80 S = type(uint80).max;
        int80 T = int80(S); //~WARN: typecasts that can truncate values should be checked
        uint72 U = type(uint72).max;
        int72 V = int72(U); //~WARN: typecasts that can truncate values should be checked
        uint64 W = type(uint64).max;
        int64 X = int64(W); //~WARN: typecasts that can truncate values should be checked
        uint56 Y = type(uint56).max;
        int56 Z = int56(Y); //~WARN: typecasts that can truncate values should be checked
        uint48 AA = type(uint48).max;
        int48 BB = int48(AA); //~WARN: typecasts that can truncate values should be checked
        uint40 CC = type(uint40).max;
        int40 DD = int40(CC); //~WARN: typecasts that can truncate values should be checked
        uint32 EE = type(uint32).max;
        int32 FF = int32(EE); //~WARN: typecasts that can truncate values should be checked
        uint24 GG = type(uint24).max;
        int24 HH = int24(GG); //~WARN: typecasts that can truncate values should be checked
        uint16 II = type(uint16).max;
        int16 JJ = int16(II); //~WARN: typecasts that can truncate values should be checked
        uint8 KK = type(uint8).max;
        int8 LL = int8(KK); //~WARN: typecasts that can truncate values should be checked
    }

    function signedUnsignedUnsafe() public pure {
        int256 a = -1;
        uint256 b = uint256(a); //~WARN: typecasts that can truncate values should be checked
        int248 c = -1;
        uint248 d = uint248(c); //~WARN: typecasts that can truncate values should be checked
        int240 e = -1;
        uint240 f = uint240(e); //~WARN: typecasts that can truncate values should be checked
        int232 g = -1;
        uint232 h = uint232(g); //~WARN: typecasts that can truncate values should be checked
        int224 i = -1;
        uint224 j = uint224(i); //~WARN: typecasts that can truncate values should be checked
        int216 k = -1;
        uint216 l = uint216(k); //~WARN: typecasts that can truncate values should be checked
        int208 m = -1;
        uint208 n = uint208(m); //~WARN: typecasts that can truncate values should be checked
        int200 o = -1;
        uint200 p = uint200(o); //~WARN: typecasts that can truncate values should be checked
        int192 q = -1;
        uint192 r = uint192(q); //~WARN: typecasts that can truncate values should be checked
        int184 s = -1;
        uint184 t = uint184(s); //~WARN: typecasts that can truncate values should be checked
        int176 u = -1;
        uint176 v = uint176(u); //~WARN: typecasts that can truncate values should be checked
        int168 w = -1;
        uint168 x = uint168(w); //~WARN: typecasts that can truncate values should be checked
        int160 y = -1;
        uint160 z = uint160(y); //~WARN: typecasts that can truncate values should be checked
        int152 A = -1;
        uint152 B = uint152(A); //~WARN: typecasts that can truncate values should be checked
        int144 C = -1;
        uint144 D = uint144(C); //~WARN: typecasts that can truncate values should be checked
        int136 E = -1;
        uint136 F = uint136(E); //~WARN: typecasts that can truncate values should be checked
        int128 G = -1;
        uint128 H = uint128(G); //~WARN: typecasts that can truncate values should be checked
        int120 I = -1;
        uint120 J = uint120(I); //~WARN: typecasts that can truncate values should be checked
        int112 K = -1;
        uint112 L = uint112(K); //~WARN: typecasts that can truncate values should be checked
        int104 M = -1;
        uint104 N = uint104(M); //~WARN: typecasts that can truncate values should be checked
        int96 O = -1;
        uint96 P = uint96(O); //~WARN: typecasts that can truncate values should be checked
        int88 Q = -1;
        uint88 R = uint88(Q); //~WARN: typecasts that can truncate values should be checked
        int80 S = -1;
        uint80 T = uint80(S); //~WARN: typecasts that can truncate values should be checked
        int72 U = -1;
        uint72 V = uint72(U); //~WARN: typecasts that can truncate values should be checked
        int64 W = -1;
        uint64 X = uint64(W); //~WARN: typecasts that can truncate values should be checked
        int56 Y = -1;
        uint56 Z = uint56(Y); //~WARN: typecasts that can truncate values should be checked
        int48 AA = -1;
        uint48 BB = uint48(AA); //~WARN: typecasts that can truncate values should be checked
        int40 CC = -1;
        uint40 DD = uint40(CC); //~WARN: typecasts that can truncate values should be checked
        int32 EE = -1;
        uint32 FF = uint32(EE); //~WARN: typecasts that can truncate values should be checked
        int24 GG = -1;
        uint24 HH = uint24(GG); //~WARN: typecasts that can truncate values should be checked
        int16 II = -1;
        uint16 JJ = uint16(II); //~WARN: typecasts that can truncate values should be checked
        int8 KK = -1;
        uint8 LL = uint8(KK); //~WARN: typecasts that can truncate values should be checked
    }

    function downcastDynamicUnsafe() public pure {
        bytes memory data = "hello world";
        bytes32 dataSlice = bytes32(data); //~WARN: typecasts that can truncate values should be checked
        string memory str = "hello world";
        bytes32 strSlice = bytes32(bytes(str)); //~WARN: typecasts that can truncate values should be checked
    }
}

contract Repros {
    function longDynamicBytesDoNotPanic() public pure {
        bytes memory stringToBytes = bytes("Initializable: contract is already initialized");
    }

    function nestedCastsAreEvaluatedAtAllDepths(uint64 a, int128 b) internal pure returns (uint64) {
        uint64 aAloneIsSafe = uint64(uint128(int128(uint128(a))));

        uint128 aPlusB = uint128(int128(uint128(a)) + b);
        //~^WARN: typecasts that can truncate values should be checked

        uint64 unsafe = uint64(aPlusB);
        //~^WARN: typecasts that can truncate values should be checked

        return uint64(uint128(int128(uint128(a)) + b));
        //~^WARN: typecasts that can truncate values should be checked
        //~|WARN: typecasts that can truncate values should be checked
    }
}
// forge-lint: disable-end(mixed-case-variable)
