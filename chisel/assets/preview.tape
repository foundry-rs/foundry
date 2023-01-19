# VHS File source
# https://github.com/charmbracelet/vhs
#
# Output:
#   Output <path>.gif               Create a GIF output at the given <path>
#   Output <path>.mp4               Create an MP4 output at the given <path>
#   Output <path>.webm              Create a WebM output at the given <path>
#
# Settings:
#   Set FontSize <number>           Set the font size of the terminal
#   Set FontFamily <string>         Set the font family of the terminal
#   Set Height <number>             Set the height of the terminal
#   Set Width <number>              Set the width of the terminal
#   Set LetterSpacing <float>       Set the font letter spacing (tracking)
#   Set LineHeight <float>          Set the font line height
#   Set Theme <string>              Set the theme of the terminal (JSON)
#   Set Padding <number>            Set the padding of the terminal
#   Set Framerate <number>          Set the framerate of the recording
#   Set PlaybackSpeed <float>       Set the playback speed of the recording
#
# Sleep:
#   Sleep <time>                    Sleep for a set amount of <time> in seconds
#
# Type:
#   Type[@<time>] "<characters>"    Type <characters> into the terminal with a
#                                   <time> delay between each character
#
# Keys:
#   Backspace[@<time>] [number]     Press the Backspace key
#   Down[@<time>] [number]          Press the Down key
#   Enter[@<time>] [number]         Press the Enter key
#   Space[@<time>] [number]         Press the Space key
#   Tab[@<time>] [number]           Press the Tab key
#   Left[@<time>] [number]          Press the Left Arrow key
#   Right[@<time>] [number]         Press the Right Arrow key
#   Up[@<time>] [number]            Press the Up Arrow key
#   Down[@<time>] [number]          Press the Down Arrow key
#   Ctrl+<key>                      Press the Control key + <key> (e.g. Ctrl+C)
#
# Display:
#   Hide                            Hide the subsequent commands from the output
#   Show                            Show the subsequent commands in the output

Output assets/preview.gif

Set FontSize 16
Set Width 1920
Set Height 1080

Type "chisel"
Enter
Sleep 500ms

Type "!help" Enter
Sleep 500ms

Type "contract Test { uint public b = 0xa57b; }"
Enter
Sleep 500ms

Type "!t"
Enter
Sleep 500ms

Type "new Test().b()"
Enter
Sleep 1.5s

Type 'address payable emptyRipe = payable(address(ripemd160(hex"")))'
Enter
Sleep 500ms

Type "vm.deal(address(this), 5 ether)"
Enter
Sleep 500ms

Type "emptyRipe.transfer(1 ether)"
Enter
Sleep 500ms

Type "address(emptyRipe).balance"
Enter
Sleep 1.5s

Hide
Type "!t"
Enter
Show

Type@5ms "function hash64(bytes32 a, bytes32 b) internal pure returns (bytes32 hash) {"
Enter
Type@5ms "    assembly {"
Enter
Type@5ms "        mstore(0x00, a)"
Enter
Type@5ms "        mstore(0x20, b)"
Enter
Type@5ms "        hash := keccak256(0x00, 0x40)"
Enter
Type@5ms "    }"
Enter
Type@5ms "}"
Enter
Sleep 500ms

Type "hash64(bytes32(0), bytes32(uint(256)))"
Enter
Sleep 1s

Type "!fork mainnet"
Enter
Sleep 500ms

Type@2ms "!fetch 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2 IWETH"
Enter
Sleep 1.5s

Type@2ms "IWETH weth = IWETH(0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2)"
Enter
Sleep 3s

Type "weth.name()"
Enter
Sleep 3s

Type "weth.balanceOf(address(weth))"
Enter
Sleep 3s

Type "!so"
Enter
Sleep 2s

Type "!s example"
Enter
Sleep 5s
