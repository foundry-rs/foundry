// config: brace_style = "allman"
contract BraceStyle
{
    struct Record
    {
        uint256 value;
    }

    enum Kind
    {
        One
    }

    function foo(uint256 x)
        public
        returns (uint256 result)
    {
        if (x > 10)
        {
            x--;
        } else
        {
            x++;
        }

        while (x > 0)
        {
            x--;
        }

        do
        {
            x++;
        } while (x < 2);

        for (uint256 i = 0; i < 2; i++)
        {
            x += i;
        }

        unchecked
    {
        }

        try this.bar({value: x}) returns (
            uint256 y
        )
        {
            x = y;
        } catch Error(string memory)
            {
            x = 0;
        } catch
            {
        }

        assembly
        {
            let y := 0
            if x
            {
                y := add(y, 1)
            }
            for
            {
                let i := 0
            } lt(i, 2)
            {
                i := add(i, 1)
            }
            {
                y := add(y, i)
            }
            switch y
            case 0
            {
                y := 1
            }
            default
            {
                y := add(y, 2)
            }
            function twice(value) -> result
            {
                result := add(value, value)
            }
        }

        return x;
    }

    function bar(uint256 value)
        public
        pure
        returns (uint256)
    {
        return value;
    }

    function empty()
        public
    {
        }
}

contract Derived is BraceStyle
{
    }
