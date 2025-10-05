# pragma version ~=0.4.3

number: public(uint256)

@external
def setNumber(newNumber: uint256):
    self.number = newNumber

@external
def increment():
    self.number += 1