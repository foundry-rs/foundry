// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/8383
contract Issue8383Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    address internal _verifier;

    mapping(bytes32 => bool) internal _vectorTested;
    mapping(bytes32 => bool) internal _vectorResult;

    function setUp() public {
        _verifier = address(new P256Verifier());
    }

    function _verifyViaVerifier(bytes32 hash, uint256 r, uint256 s, uint256 x, uint256 y) internal returns (bool) {
        return _verifyViaVerifier(hash, bytes32(r), bytes32(s), bytes32(x), bytes32(y));
    }

    function _verifyViaVerifier(bytes32 hash, bytes32 r, bytes32 s, bytes32 x, bytes32 y) internal returns (bool) {
        bytes memory payload = abi.encode(hash, r, s, x, y);
        if (uint256(y) & 0xff == 0) {
            bytes memory truncatedPayload = abi.encodePacked(hash, r, s, x, bytes31(y));
            _verifierCall(truncatedPayload);
        }
        if (uint256(keccak256(abi.encode(payload, "1"))) & 0x1f == 0) {
            uint256 r = uint256(keccak256(abi.encode(payload, "2")));
            payload = abi.encodePacked(payload, new bytes(r & 0xff));
        }
        bytes32 payloadHash = keccak256(payload);
        if (_vectorTested[payloadHash]) return _vectorResult[payloadHash];
        _vectorTested[payloadHash] = true;
        return (_vectorResult[payloadHash] = _verifierCall(payload));
    }

    function _verifierCall(bytes memory payload) internal returns (bool) {
        (bool success, bytes memory result) = _verifier.call(payload);
        return abi.decode(result, (bool));
    }

    function testP256VerifyOutOfBounds() public {
        vm.pauseGasMetering();
        uint256 p = 0xFFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF;
        _verifyViaVerifier(bytes32(0), 1, 1, 1, 1);
        _verifyViaVerifier(bytes32(0), 1, 1, 0, 1);
        _verifyViaVerifier(bytes32(0), 1, 1, 1, 0);
        _verifyViaVerifier(bytes32(0), 1, 1, 1, p);
        _verifyViaVerifier(bytes32(0), 1, 1, p, 1);
        _verifyViaVerifier(bytes32(0), 1, 1, p - 1, 1);
        vm.resumeGasMetering();
    }
}

contract P256Verifier {
    uint256 private constant GX = 0x6B17D1F2E12C4247F8BCE6E563A440F277037D812DEB33A0F4A13945D898C296;
    uint256 private constant GY = 0x4FE342E2FE1A7F9B8EE7EB4A7C0F9E162BCE33576B315ECECBB6406837BF51F5;
    uint256 private constant P = 0xFFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF; // `A = P - 3`.
    uint256 private constant N = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551;
    uint256 private constant B = 0x5AC635D8AA3A93E7B3EBBD55769886BC651D06B0CC53B0F63BCE3C3E27D2604B;

    fallback() external payable {
        assembly {
            // For this implementation, we will use the memory without caring about
            // the free memory pointer or zero pointer.
            // The slots `0x00`, `0x20`, `0x40`, `0x60`, will not be accessed for the `Points[16]` array,
            // and can be used for storing other variables.

            mstore(0x40, P) // Set `0x40` to `P`.

            function jAdd(x1, y1, z1, x2, y2, z2) -> x3, y3, z3 {
                if iszero(z1) {
                    x3 := x2
                    y3 := y2
                    z3 := z2
                    leave
                }
                if iszero(z2) {
                    x3 := x1
                    y3 := y1
                    z3 := z1
                    leave
                }
                let p := mload(0x40)
                let zz1 := mulmod(z1, z1, p)
                let zz2 := mulmod(z2, z2, p)
                let u1 := mulmod(x1, zz2, p)
                let u2 := mulmod(x2, zz1, p)
                let s1 := mulmod(y1, mulmod(zz2, z2, p), p)
                let s2 := mulmod(y2, mulmod(zz1, z1, p), p)
                let h := addmod(u2, sub(p, u1), p)
                let hh := mulmod(h, h, p)
                let hhh := mulmod(h, hh, p)
                let r := addmod(s2, sub(p, s1), p)
                x3 := addmod(addmod(mulmod(r, r, p), sub(p, hhh), p), sub(p, mulmod(2, mulmod(u1, hh, p), p)), p)
                y3 := addmod(mulmod(r, addmod(mulmod(u1, hh, p), sub(p, x3), p), p), sub(p, mulmod(s1, hhh, p)), p)
                z3 := mulmod(h, mulmod(z1, z2, p), p)
            }

            function setJPoint(i, x, y, z) {
                // We will multiply by `0x80` (i.e. `shl(7, i)`) instead
                // since the memory expansion costs are cheaper than doing `mul(0x60, i)`.
                // Also help combine the lookup expression for `u1` and `u2` in `jMultShamir`.
                i := shl(7, i)
                mstore(i, x)
                mstore(add(i, returndatasize()), y)
                mstore(add(i, 0x40), z)
            }

            function setJPointDouble(i, j) {
                j := shl(7, j)
                let x := mload(j)
                let y := mload(add(j, returndatasize()))
                let z := mload(add(j, 0x40))
                let p := mload(0x40)
                let yy := mulmod(y, y, p)
                let zz := mulmod(z, z, p)
                let s := mulmod(4, mulmod(x, yy, p), p)
                let m := addmod(mulmod(3, mulmod(x, x, p), p), mulmod(mload(returndatasize()), mulmod(zz, zz, p), p), p)
                let x2 := addmod(mulmod(m, m, p), sub(p, mulmod(2, s, p)), p)
                let y2 := addmod(mulmod(m, addmod(s, sub(p, x2), p), p), sub(p, mulmod(8, mulmod(yy, yy, p), p)), p)
                let z2 := mulmod(2, mulmod(y, z, p), p)
                setJPoint(i, x2, y2, z2)
            }

            function setJPointAdd(i, j, k) {
                j := shl(7, j)
                k := shl(7, k)
                let x, y, z :=
                    jAdd(
                        mload(j),
                        mload(add(j, returndatasize())),
                        mload(add(j, 0x40)),
                        mload(k),
                        mload(add(k, returndatasize())),
                        mload(add(k, 0x40))
                    )
                setJPoint(i, x, y, z)
            }

            let r := calldataload(0x20)
            let n := N

            {
                let s := calldataload(0x40)
                if lt(shr(1, n), s) { s := sub(n, s) }

                // Perform `modExp(s, N - 2, N)`.
                // After which, we can abuse `returndatasize()` to get `0x20`.
                mstore(0x800, 0x20)
                mstore(0x820, 0x20)
                mstore(0x840, 0x20)
                mstore(0x860, s)
                mstore(0x880, sub(n, 2))
                mstore(0x8a0, n)

                let p := mload(0x40)
                mstore(0x20, xor(3, p)) // Set `0x20` to `A`.
                let Qx := calldataload(0x60)
                let Qy := calldataload(0x80)

                if iszero(
                    and( // The arguments of `and` are evaluated last to first.
                        and(
                            and(gt(calldatasize(), 0x9f), and(lt(iszero(r), lt(r, n)), lt(iszero(s), lt(s, n)))),
                            eq(
                                mulmod(Qy, Qy, p),
                                addmod(mulmod(addmod(mulmod(Qx, Qx, p), mload(returndatasize()), p), Qx, p), B, p)
                            )
                        ),
                        and(
                            // We need to check that the `returndatasize` is indeed 32,
                            // so that we can return false if the chain does not have the modexp precompile.
                            eq(returndatasize(), 0x20),
                            staticcall(gas(), 0x05, 0x800, 0xc0, returndatasize(), 0x20)
                        )
                    )
                ) {
                    // POC Note:
                    // Changing this to `return(0x80, 0x20)` fixes it.
                    // Alternatively, adding `if mload(0x8c0) { invalid() }` just before the return also fixes it.
                    return(0x8c0, 0x20)
                }

                setJPoint(0x01, Qx, Qy, 1)
                setJPoint(0x04, GX, GY, 1)
                setJPointDouble(0x02, 0x01)
                setJPointDouble(0x08, 0x04)
                setJPointAdd(0x03, 0x01, 0x02)
                setJPointAdd(0x05, 0x01, 0x04)
                setJPointAdd(0x06, 0x02, 0x04)
                setJPointAdd(0x07, 0x03, 0x04)
                setJPointAdd(0x09, 0x01, 0x08)
                setJPointAdd(0x0a, 0x02, 0x08)
                setJPointAdd(0x0b, 0x03, 0x08)
                setJPointAdd(0x0c, 0x04, 0x08)
                setJPointAdd(0x0d, 0x01, 0x0c)
                setJPointAdd(0x0e, 0x02, 0x0c)
                setJPointAdd(0x0f, 0x03, 0x0c)
            }

            let i := 0
            let u1 := mulmod(calldataload(0x00), mload(0x00), n)
            let u2 := mulmod(r, mload(0x00), n)
            let y := 0
            let z := 0
            let x := 0
            let p := mload(0x40)
            for {} 1 {} {
                if z {
                    let yy := mulmod(y, y, p)
                    let zz := mulmod(z, z, p)
                    let s := mulmod(4, mulmod(x, yy, p), p)
                    let m :=
                        addmod(mulmod(3, mulmod(x, x, p), p), mulmod(mload(returndatasize()), mulmod(zz, zz, p), p), p)
                    let x2 := addmod(mulmod(m, m, p), sub(p, mulmod(2, s, p)), p)
                    let y2 := addmod(mulmod(m, addmod(s, sub(p, x2), p), p), sub(p, mulmod(8, mulmod(yy, yy, p), p)), p)
                    let z2 := mulmod(2, mulmod(y, z, p), p)
                    yy := mulmod(y2, y2, p)
                    zz := mulmod(z2, z2, p)
                    s := mulmod(4, mulmod(x2, yy, p), p)
                    m :=
                        addmod(mulmod(3, mulmod(x2, x2, p), p), mulmod(mload(returndatasize()), mulmod(zz, zz, p), p), p)
                    x := addmod(mulmod(m, m, p), sub(p, mulmod(2, s, p)), p)
                    z := mulmod(2, mulmod(y2, z2, p), p)
                    y := addmod(mulmod(m, addmod(s, sub(p, x), p), p), sub(p, mulmod(8, mulmod(yy, yy, p), p)), p)
                }
                for { let o := or(and(shr(245, shl(i, u1)), 0x600), and(shr(247, shl(i, u2)), 0x180)) } o {} {
                    let z2 := mload(add(o, 0x40))
                    if iszero(z2) { break }
                    if iszero(z) {
                        x := mload(o)
                        y := mload(add(o, returndatasize()))
                        z := z2
                        break
                    }
                    let zz1 := mulmod(z, z, p)
                    let zz2 := mulmod(z2, z2, p)
                    let u1_ := mulmod(x, zz2, p)
                    let s1 := mulmod(y, mulmod(zz2, z2, p), p)
                    let h := addmod(mulmod(mload(o), zz1, p), sub(p, u1_), p)
                    let hh := mulmod(h, h, p)
                    let hhh := mulmod(h, hh, p)
                    let r_ := addmod(mulmod(mload(add(o, returndatasize())), mulmod(zz1, z, p), p), sub(p, s1), p)
                    x := addmod(addmod(mulmod(r_, r_, p), sub(p, hhh), p), sub(p, mulmod(2, mulmod(u1_, hh, p), p)), p)
                    y := addmod(mulmod(r_, addmod(mulmod(u1_, hh, p), sub(p, x), p), p), sub(p, mulmod(s1, hhh, p)), p)
                    z := mulmod(h, mulmod(z, z2, p), p)
                    break
                }
                // Just unroll twice. Fully unrolling will only save around 1% to 2% gas, but make the
                // bytecode very bloated, which may incur more runtime costs after Verkle.
                // See: https://notes.ethereum.org/%40vbuterin/verkle_tree_eip
                // It's very unlikely that Verkle will come before the P256 precompile. But who knows?
                if z {
                    let yy := mulmod(y, y, p)
                    let zz := mulmod(z, z, p)
                    let s := mulmod(4, mulmod(x, yy, p), p)
                    let m :=
                        addmod(mulmod(3, mulmod(x, x, p), p), mulmod(mload(returndatasize()), mulmod(zz, zz, p), p), p)
                    let x2 := addmod(mulmod(m, m, p), sub(p, mulmod(2, s, p)), p)
                    let y2 := addmod(mulmod(m, addmod(s, sub(p, x2), p), p), sub(p, mulmod(8, mulmod(yy, yy, p), p)), p)
                    let z2 := mulmod(2, mulmod(y, z, p), p)
                    yy := mulmod(y2, y2, p)
                    zz := mulmod(z2, z2, p)
                    s := mulmod(4, mulmod(x2, yy, p), p)
                    m :=
                        addmod(mulmod(3, mulmod(x2, x2, p), p), mulmod(mload(returndatasize()), mulmod(zz, zz, p), p), p)
                    x := addmod(mulmod(m, m, p), sub(p, mulmod(2, s, p)), p)
                    z := mulmod(2, mulmod(y2, z2, p), p)
                    y := addmod(mulmod(m, addmod(s, sub(p, x), p), p), sub(p, mulmod(8, mulmod(yy, yy, p), p)), p)
                }
                for { let o := or(and(shr(243, shl(i, u1)), 0x600), and(shr(245, shl(i, u2)), 0x180)) } o {} {
                    let z2 := mload(add(o, 0x40))
                    if iszero(z2) { break }
                    if iszero(z) {
                        x := mload(o)
                        y := mload(add(o, returndatasize()))
                        z := z2
                        break
                    }
                    let zz1 := mulmod(z, z, p)
                    let zz2 := mulmod(z2, z2, p)
                    let u1_ := mulmod(x, zz2, p)
                    let s1 := mulmod(y, mulmod(zz2, z2, p), p)
                    let h := addmod(mulmod(mload(o), zz1, p), sub(p, u1_), p)
                    let hh := mulmod(h, h, p)
                    let hhh := mulmod(h, hh, p)
                    let r_ := addmod(mulmod(mload(add(o, returndatasize())), mulmod(zz1, z, p), p), sub(p, s1), p)
                    x := addmod(addmod(mulmod(r_, r_, p), sub(p, hhh), p), sub(p, mulmod(2, mulmod(u1_, hh, p), p)), p)
                    y := addmod(mulmod(r_, addmod(mulmod(u1_, hh, p), sub(p, x), p), p), sub(p, mulmod(s1, hhh, p)), p)
                    z := mulmod(h, mulmod(z, z2, p), p)
                    break
                }
                i := add(i, 4)
                if eq(i, 256) { break }
            }

            if iszero(z) {
                mstore(returndatasize(), iszero(r))
                return(returndatasize(), 0x20)
            }

            // Perform `modExp(z, P - 2, P)`.
            // `0x800`, `0x820, `0x840` are still set to `0x20`.
            mstore(0x860, z)
            mstore(0x880, sub(p, 2))
            mstore(0x8a0, p)

            mstore(
                returndatasize(),
                and( // The arguments of `and` are evaluated last to first.
                    eq(mod(mulmod(x, mulmod(mload(returndatasize()), mload(returndatasize()), p), p), n), r),
                    staticcall(gas(), 0x05, 0x800, 0xc0, returndatasize(), returndatasize())
                )
            )
            return(returndatasize(), returndatasize())
        }
    }
}
