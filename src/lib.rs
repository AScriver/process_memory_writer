/*!
 * process_memory_writer.rs
 *
 * This module provides the `MemoryWriter` class, which allows reading and writing memory
 * of another process on Windows systems via a Python extension using PyO3 and Rust.
 *
 * # Overview
 *
 * The `MemoryWriter` class enables users to:
 *
 * - **Open a process** by its executable name.
 * - **Read memory** from the target process.
 * - **Write memory** to the target process.
 * - **Continuously write data** to a specific memory address in the process.
 *
 * # Safety and Security Considerations
 *
 * - Modifying another process's memory can be dangerous and may cause system instability.
 * - Ensure that you have the necessary permissions to manipulate the target process.
 * - Be aware of legal implications and software licenses when using this functionality.
 */

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use windows::Win32::Foundation::{CloseHandle, HANDLE, MAX_PATH};
use windows::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_VM_OPERATION, PROCESS_VM_WRITE, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};
use windows::Win32::System::ProcessStatus::{GetModuleBaseNameW, K32EnumProcesses};

/// A wrapper around a Windows `HANDLE` that can be safely sent across threads.
///
/// This struct ensures that the `HANDLE` is properly closed when it goes out of scope,
/// preventing resource leaks.
struct SendableHandle(HANDLE);

unsafe impl Send for SendableHandle {}
unsafe impl Sync for SendableHandle {}

impl SendableHandle {
    /// Creates a new `SendableHandle` from a `HANDLE`.
    ///
    /// # Arguments
    ///
    /// * `handle` - The Windows `HANDLE` to wrap.
    fn new(handle: HANDLE) -> Self {
        SendableHandle(handle)
    }
}

impl Drop for SendableHandle {
    /// Closes the `HANDLE` when the `SendableHandle` is dropped.
    ///
    /// This ensures that the handle does not remain open longer than necessary,
    /// preventing potential resource leaks.
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

impl Clone for SendableHandle {
    /// Clones the `SendableHandle`, duplicating the `HANDLE`.
    ///
    /// **Note**: This does not create a new `HANDLE`; it copies the existing `HANDLE` value.
    /// The underlying handle is shared between clones.
    fn clone(&self) -> Self {
        SendableHandle(self.0)
    }
}

#[derive(FromPyObject)]
enum ProcessIdentifier {
    Pid(u32),
    Name(String),
}

/// A Python class that allows reading and writing memory of another process.
///
/// The `MemoryWriter` class provides methods to:
///
/// - Open a process by name.
/// - Start and stop continuous memory writing.
/// - Set memory data to write.
/// - Read memory from the process.
#[pyclass]
struct MemoryWriter {
    /// Handle to the target process.
    h_process: Option<Arc<SendableHandle>>,
    /// Atomic flag indicating whether the worker thread should continue running.
    running: Arc<AtomicBool>,
    /// The memory address in the target process where data will be written.
    address: Arc<AtomicU64>,
    /// The data to be written to the target process's memory.
    data: Arc<Mutex<Vec<u8>>>,
    /// The worker thread that continuously writes data to the target process's memory.
    worker: Option<std::thread::JoinHandle<()>>,
}

#[pymethods]
impl MemoryWriter {
    /// Creates a new instance of `MemoryWriter`.
    ///
    /// # Returns
    ///
    /// A new `MemoryWriter` object.
    #[new]
    fn new() -> Self {
        MemoryWriter {
            h_process: None,
            running: Arc::new(AtomicBool::new(false)),
            address: Arc::new(AtomicU64::new(0)),
            data: Arc::new(Mutex::new(Vec::new())),
            worker: None,
        }
    }

    /// Opens a process by its PID or executable name.
    ///
    /// # Arguments
    ///
    /// * `process_name` - An name or PID of the process to open (e.g., `"notepad.exe"`).
    ///
    /// # Returns
    ///
    /// * `Ok(true)` if the process was successfully opened.
    /// * `Ok(false)` if the process was not found or could not be opened.
    /// * `Err` if an unexpected error occurred.
    ///
    /// # Example
    ///
    /// ```python
    /// writer = MemoryWriter()
    /// success = writer.open_process(pid=1234, process_name=None)
    /// if success:
    ///     print("Process opened successfully")
    /// else:
    ///     print("Failed to open process")
    /// ```
    fn open_process(&mut self, process_name: ProcessIdentifier) -> PyResult<bool> {
        match process_name {
            ProcessIdentifier::Pid(pid) => self.open_process_by_pid(pid),
            ProcessIdentifier::Name(name) => self.open_process_by_name(name),
        }
    }

    fn open_process_by_pid(&mut self, pid: u32) -> PyResult<bool> {
        unsafe {
            let handle = match OpenProcess(PROCESS_VM_WRITE | PROCESS_VM_OPERATION, false, pid) {
                Ok(handle) => handle,
                Err(_) => return Ok(false),
            };
            self.h_process = Some(Arc::new(SendableHandle::new(handle)));
            Ok(true)
        }
    }

    fn open_process_by_name(&mut self, process_name: String) -> PyResult<bool> {
        unsafe {
            let mut process_ids = vec![0u32; 1024];
            let mut bytes_returned = 0u32;
    
            if K32EnumProcesses(process_ids.as_mut_ptr(), (process_ids.len() * std::mem::size_of::<u32>()) as u32, &mut bytes_returned).as_bool(){
                let num_processes = bytes_returned as usize / std::mem::size_of::<u32>();
                process_ids.truncate(num_processes);
    
                for pid in process_ids {
                    let handle_result = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid);
                    let handle = handle_result.unwrap();
                    if handle.is_invalid() {
                        continue;
                    }
    
                    let mut process_name_buf = [0u16; MAX_PATH as usize];
                    let name_len = GetModuleBaseNameW(handle, None, &mut process_name_buf);
    
                    if name_len > 0 {
                        let name = String::from_utf16_lossy(&process_name_buf[..name_len as usize]);
                        if name.eq_ignore_ascii_case(&process_name) {
                            self.h_process = Some(Arc::new(SendableHandle::new(handle)));
                            return Ok(true);
                        }
                    }
    
                    let _ = CloseHandle(handle);
                }
            }
    
            Ok(false)
        }
    }

    /// Starts the worker thread that continuously writes data to the target process's memory.
    ///
    /// The worker thread will repeatedly write the data set by `set_memory_data` to the
    /// memory address specified, until `stop` is called.
    ///
    /// # Example
    ///
    /// ```python
    /// writer.start()
    /// ```
    fn start(&mut self) {
        if let Some(h_process_arc) = &self.h_process {
            let running = self.running.clone();
            let h_process = h_process_arc.clone();
            let address = self.address.clone();
            let data = self.data.clone();

            // Set the running flag to true.
            running.store(true, Ordering::SeqCst);

            // Spawn the worker thread.
            self.worker = Some(thread::spawn(move || {
                let h_process = h_process.0;

                while running.load(Ordering::SeqCst) {
                    let addr = address.load(Ordering::SeqCst);
                    let buf = {
                        let data_lock = data.lock().unwrap();
                        data_lock.clone()
                    };

                    unsafe {
                        let mut bytes_written = 0;
                        // Write data to the process's memory.
                        if WriteProcessMemory(
                            h_process,
                            addr as _,
                            buf.as_ptr() as _,
                            buf.len(),
                            Some(&mut bytes_written),
                        )
                        .is_err()
                        {
                            // If writing fails, exit the loop.
                            break;
                        }
                    }
                }
            }));
        }
    }

    /// Stops the worker thread that is writing data to the process's memory.
    ///
    /// # Example
    ///
    /// ```python
    /// writer.stop()
    /// ```
    fn stop(&mut self) {
        // Set the running flag to false to signal the worker thread to stop.
        self.running.store(false, Ordering::SeqCst);

        // Wait for the worker thread to finish.
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }

    /// Sets the memory address and data to be written to the target process.
    ///
    /// # Arguments
    ///
    /// * `new_address` - The memory address in the target process where data will be written.
    /// * `new_data` - A bytes-like object containing the data to write.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the address and data were successfully set.
    /// * `Err` if an error occurred.
    ///
    /// # Example
    ///
    /// ```python
    /// writer.set_memory_data(0x12345678, b'\x90\x90\x90\x90')
    /// ```
    fn set_memory_data(&self, address: usize, data: &[u8]) -> PyResult<()> {
        // Update the address and data.
        self.address.store(address as u64, Ordering::SeqCst);
        let mut current_data = self.data.lock().unwrap();
        *current_data = data.to_vec();
        Ok(())
    }

    /// Reads memory from the target process at the specified address and size.
    ///
    /// # Arguments
    ///
    /// * `address` - The memory address in the target process to read from.
    /// * `size` - The number of bytes to read.
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<u8>)` containing the bytes read from the process's memory.
    /// * `Err` if the process handle is not available or reading memory failed.
    ///
    /// # Example
    ///
    /// ```python
    /// data = writer.read_memory(0x12345678, 4)
    /// print(data)
    /// ```
    fn read_memory(&self, address: usize, size: usize) -> PyResult<Vec<u8>> {
        // Ensure the process handle is available.
        let h_process = self
            .h_process
            .as_ref()
            .ok_or_else(|| PyErr::new::<PyException, _>("Process handle is not available"))?;
        let mut buffer = vec![0u8; size];
        unsafe {
            ReadProcessMemory(
                h_process.0,
                address as _,
                buffer.as_mut_ptr() as _,
                size,
                None,
            )
            .map_err(|e| PyErr::new::<PyException, _>(format!("Failed to read memory: {}", e)))?;
        }
        Ok(buffer)
    }
}

/// The Python module definition.
///
/// This module provides the `MemoryWriter` class to Python.
///
/// # Example
///
/// ```python
/// from process_memory_writer import MemoryWriter
///
/// writer = MemoryWriter()
/// if writer.open_process("notepad.exe"):
///     writer.set_memory_data(0x12345678, b'\x90\x90')
///     writer.start()
///     # Do other work...
///     writer.stop()
/// else:
///     print("Failed to open process")
/// ```
#[pymodule]
fn process_memory_writer(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<MemoryWriter>()?;
    Ok(())
}
