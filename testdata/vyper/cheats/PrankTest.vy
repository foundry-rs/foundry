import Vm as Vm
from . import PrankTest

vm: constant(address) = 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D

@external
def test_prank_simple(sender: address):
    Vm(vm).startPrank(sender)
    PrankTest(self).assert_sender(sender)

@external
def test_prank_with_origin(sender: address, origin: address):
    Vm(vm).startPrank(sender, origin)
    PrankTest(self).assert_sender(sender)
    PrankTest(self).assert_origin(sender)

@external
def assert_sender(expected_sender: address):
    Vm(vm).assertEq(msg.sender, expected_sender)

@external
def assert_origin(expected_sender: address):
    Vm(vm).assertEq(msg.sender, expected_sender)