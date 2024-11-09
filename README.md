# Process Memory Writer

`process_memory_writer` is a Python extension written in Rust using PyO3. It allows reading and writing memory of another process on Windows systems.

## Overview

The `MemoryWriter` class enables users to:

- **Open a process** by its executable name or PID.
- **Read memory** from the target process.
- **Write memory** to the target process.
- **Continuously write data** to a specific memory address in the process.

## Safety and Security Considerations

- Modifying another process's memory can be dangerous and may cause system instability.
- Ensure that you have the necessary permissions to manipulate the target process.
- Be aware of legal implications and software licenses when using this functionality.

## Installation

1. Ensure you have Rust and Python installed on your system.
2. Clone the repository.
3. Navigate to the project directory.
4. Create a virtual environment and activate it:
    ```sh
    python -m venv pyo3
    source pyo3/Scripts/activate  # On Windows
    ```
5. Install the required dependencies:
    ```sh
    pip install -r requirements.txt
    ```
6. Build the project using Maturin:
    ```sh
    maturin develop
    ```

## Usage

Here is an example of how to use the `MemoryWriter` class:

```python
from process_memory_writer import MemoryWriter

# Create a new MemoryWriter instance
writer = MemoryWriter()

# Open a process by name
success = writer.open_process("notepad.exe")
if success:
    print("Process opened successfully")
else:
    print("Failed to open process")

# Open a process by PID
success = writer.open_process(1234)
if success:
    print("Process opened successfully")
else:
    print("Failed to open process")

# Set memory data to write
writer.set_memory_data(0x12345678, b"data")

# Start continuous memory writing
writer.start()

# Stop continuous memory writing
writer.stop()
```