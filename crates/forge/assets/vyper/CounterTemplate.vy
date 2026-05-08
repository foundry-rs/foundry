# pragma version ~=0.5.0a1

number: public(uint256)

@external
def setNumber(newNumber: uint256):
    self.number = newNumber

@external
def increment():
    self.number += 1
