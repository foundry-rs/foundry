from . import ICounter

interface Vm:
    def deployCode(artifact_name: String[1024], args: Bytes[1024] = b"") -> address: nonpayable

vm: constant(Vm) = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D)
counter: ICounter

@external
def setUp():
    self.counter = ICounter(extcall vm.deployCode("vyper/Counter.vy"))

@external
def test_increment():
    extcall self.counter.increment()
    assert staticcall self.counter.number() == 1
