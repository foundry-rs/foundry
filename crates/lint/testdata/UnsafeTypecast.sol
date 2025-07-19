contract UnsafeTypecast {
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