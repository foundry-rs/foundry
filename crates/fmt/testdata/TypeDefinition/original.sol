pragma solidity ^0.8.8;

    type Hello is uint;

contract TypeDefinition {
    event Moon(Hello world);

        function demo(Hello world) public {
        world = Hello.wrap(Hello.unwrap(world) + 1337);
        emit Moon(world);
    }
}
