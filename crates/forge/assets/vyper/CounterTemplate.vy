counter: public(uint256)

@deploy
@payable
def __init__(initial_counter: uint256):
    self.counter = initial_counter

@external
def set_counter(new_counter: uint256):
    self.counter = new_counter

@external
def increment():
    self.counter += 1
