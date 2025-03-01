from typing import List, Optional, Union
from warnings import deprecated

class Function:
    """
    Represents a public smart contract function.

    Attributes:
        selector (str): Function selector as a 4-byte hex string without '0x' prefix (e.g., 'aabbccdd').
        bytecode_offset (int): Starting byte offset within the EVM bytecode for the function body.
        arguments (Optional[str]): Function argument types in canonical format (e.g., 'uint256,address[]').
            None if arguments were not extracted
        state_mutability (Optional[str]): Function's state mutability ('pure', 'view', 'payable', or 'nonpayable').
            None if state mutability was not extracted
    """

    selector: str
    bytecode_offset: int
    arguments: Optional[str]
    state_mutability: Optional[str]

class StorageRecord:
    """
    Represents a storage variable record in a smart contract's storage layout.

    Attributes:
        slot (str): Storage slot number as a hex string (e.g., '0', '1b').
        offset (int): Byte offset within the storage slot (0-31).
        type (str): Variable type (e.g., 'uint256', 'mapping(address => uint256)', 'bytes32').
        reads (List[str]): List of function selectors that read from this storage location.
        writes (List[str]): List of function selectors that write to this storage location.
    """

    slot: str
    offset: int
    type: str
    reads: List[str]
    writes: List[str]

class Contract:
    """
    Contains analyzed information about a smart contract.

    Attributes:
        functions (Optional[List[Function]]): List of detected contract functions.
            None if no functions were extracted
        storage (Optional[List[StorageRecord]]): List of contract storage records.
            None if storage layout was not extracted
    """

    functions: Optional[List[Function]]
    storage: Optional[List[StorageRecord]]

def contract_info(
    code: Union[bytes, str],
    *,
    selectors: bool = False,
    arguments: bool = False,
    state_mutability: bool = False,
    storage: bool = False,
) -> Contract:
    """
    Extracts information about a smart contract from its EVM bytecode.

    Args:
        code (Union[bytes, str]): Runtime bytecode as a hex string (with or without '0x' prefix)
            or raw bytes.
        selectors (bool, optional): When True, extracts function selectors. Defaults to False.
        arguments (bool, optional): When True, extracts function arguments. Defaults to False.
        state_mutability (bool, optional): When True, extracts function state mutability.
            Defaults to False.
        storage (bool, optional): When True, extracts the contract's storage layout.
            Defaults to False.

    Returns:
        Contract: Object containing the requested smart contract information. Fields that
            weren't requested to be extracted will be None.
    """
    ...

@deprecated("Use contract_info() with selectors=True instead")
def function_selectors(code: Union[bytes, str], gas_limit: int = 500000) -> List[str]:
    """
    Extracts function selectors from the given bytecode.

    Args:
        code (Union[bytes, str]): Runtime bytecode as a hex string or bytes.
        gas_limit (int, optional): Maximum gas to use. Defaults to 500000.

    Returns:
        List[str]: List of selectors encoded as hex strings.
    """
    ...

@deprecated("Use contract_info() with arguments=True instead")
def function_arguments(
    code: Union[bytes, str], selector: Union[bytes, str], gas_limit: int = 50000
) -> str:
    """
    Extracts function arguments for a given selector from the bytecode.

    Args:
        code (Union[bytes, str]): Runtime bytecode as a hex string or bytes.
        selector (Union[bytes, str]): Function selector as a hex string or bytes.
        gas_limit (int, optional): Maximum gas to use. Defaults to 50000.

    Returns:
        str: Arguments of the function.
    """
    ...

@deprecated("Use contract_info() with state_mutability=True instead")
def function_state_mutability(
    code: Union[bytes, str], selector: Union[bytes, str], gas_limit: int = 500000
) -> str:
    """
    Extracts function state mutability for a given selector from the bytecode.

    Args:
        code (Union[bytes, str]): Runtime bytecode as a hex string or bytes.
        selector (Union[bytes, str]): Function selector as a hex string or bytes.
        gas_limit (int, optional): Maximum gas to use. Defaults to 500000.

    Returns:
        str: "payable" | "nonpayable" | "view" | "pure"
    """
    ...
