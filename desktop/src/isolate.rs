//! Analyses run their language model in a separate process that can't reach the network: the app's own
//! program started as a worker, which talks to the app only through its standard input and output.
//!
//! A model is data, but it's read by llama.cpp, a large C++ library; a model file made to exploit a bug in
//! it could run code. In the app's process that code could do whatever the app can, which includes going
//! online. In the worker it can't: on Windows the worker runs in an AppContainer with no capabilities (no
//! internet, no local network, and only the files it's given: the program and the model); on the Mac,
//! under the system sandbox with networking denied. Nothing is assumed: the worker tries to connect to an
//! address that is never routed, and the model is loaded only when that is refused as not allowed. If the
//! sandbox can't be set up, analyses don't run here.
//!
//! The worker answers one request at a time, as a line of JSON each way. Cancelling ends the worker; the
//! next analysis starts a new one.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use stv_core::analysis::{Model, CANCELLED};

/// The argument that starts the program as the worker.
pub const ARG: &str = "--analysis-worker";

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Request {
    Probe,
    Load { path: String, ctx: u32, threads: i32, gpu: bool },
    Count { text: String },
    Complete {
        system: String,
        user: String,
        grammar: String,
        max_tokens: usize,
        /// Send each piece of the answer as it's written, as `Text` replies before `Done`.
        #[serde(default)]
        stream: bool,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Reply {
    Probe { isolated: bool, detail: String },
    Loaded { context: usize },
    Count { n: usize },
    Done { text: String },
    /// A piece of an answer still being written.
    Text { text: String },
    Error { message: String },
}

/// Whether this process can reach a network: it tries an address reserved for documentation (TEST-NET-1),
/// which no network routes, so nothing leaves the computer either way. Refused as not allowed: sandboxed.
/// Anything else (no answer, refused by the network) means networking works here.
pub fn probe() -> (bool, String) {
    use std::net::{SocketAddr, TcpStream};
    let address: SocketAddr = "192.0.2.1:9".parse().expect("an address");
    let (isolated, detail) = match TcpStream::connect_timeout(&address, Duration::from_millis(500)) {
        Ok(_) => (false, "a connection was made".into()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => (true, e.to_string()),
        Err(e) => (false, e.to_string()),
    };
    // For testing the sandbox only (examples/isolation_check.rs): whether a name can be looked up.
    match std::env::var("STV_PROBE_DNS") {
        Ok(name) => {
            use std::net::ToSocketAddrs;
            let looked_up = match (name.as_str(), 443).to_socket_addrs() {
                Ok(found) => format!("looked up {name}: {:?}", found.collect::<Vec<_>>()),
                Err(e) => format!("couldn't look up {name}: {e}"),
            };
            (isolated, format!("{detail}; {looked_up}"))
        }
        Err(_) => (isolated, detail),
    }
}

/// The worker: answers requests on standard input until it closes.
pub fn serve() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut model: Option<crate::llm::Local> = None;
    let never = Arc::new(AtomicBool::new(false));
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let reply = match serde_json::from_str::<Request>(&line) {
            Err(e) => Reply::Error { message: format!("Bad request: {e}") },
            Ok(Request::Probe) => {
                let (isolated, detail) = probe();
                Reply::Probe { isolated, detail }
            }
            Ok(Request::Load { path, ctx, threads, gpu }) => {
                // Checked again here, so a worker started some other way won't load a model online.
                let (isolated, detail) = probe();
                if !isolated {
                    Reply::Error { message: format!("This process can reach the network ({detail}), so the model isn't loaded in it.") }
                } else {
                    match crate::llm::Local::load(Path::new(&path), ctx, threads, gpu, never.clone()) {
                        Ok(m) => {
                            let context = m.context();
                            model = Some(m);
                            Reply::Loaded { context }
                        }
                        Err(message) => Reply::Error { message },
                    }
                }
            }
            Ok(Request::Count { text }) => match &model {
                Some(m) => Reply::Count { n: m.count(&text) },
                None => Reply::Error { message: "No model is loaded.".into() },
            },
            Ok(Request::Complete { system, user, grammar, max_tokens, stream }) => match model.as_mut() {
                Some(m) => {
                    let mut send = |text: &str| {
                        if stream {
                            if let Ok(line) = serde_json::to_string(&Reply::Text { text: text.into() }) {
                                let _ = writeln!(stdout, "{line}").and_then(|_| stdout.flush());
                            }
                        }
                    };
                    match m.stream(&system, &user, &grammar, &serde_json::Value::Null, max_tokens, &mut send) {
                        Ok(text) => Reply::Done { text },
                        Err(message) => Reply::Error { message },
                    }
                }
                None => Reply::Error { message: "No model is loaded.".into() },
            },
        };
        let Ok(text) = serde_json::to_string(&reply) else { break };
        if writeln!(stdout, "{text}").and_then(|_| stdout.flush()).is_err() {
            break;
        }
    }
}

/// A running worker, from the app's side.
struct Worker {
    input: Box<dyn Write + Send>,
    lines: mpsc::Receiver<String>,
    process: sandbox::Process,
}

impl Worker {
    fn start() -> Result<Self, String> {
        let (input, output, process) = sandbox::spawn()?;
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self { input, lines, process })
    }
}

struct Inner {
    worker: Option<Worker>,
}

/// Starts a worker and asks it whether it can reach the network, for testing the sandbox.
pub fn check() -> Result<(bool, String), String> {
    let mut worker = Worker::start()?;
    writeln!(worker.input, "{}", serde_json::to_string(&Request::Probe).expect("json")).map_err(|e| e.to_string())?;
    let line = worker.lines.recv_timeout(Duration::from_secs(20)).map_err(|_| "No answer from the worker.".to_string())?;
    match serde_json::from_str(&line).map_err(|e| e.to_string())? {
        Reply::Probe { isolated, detail } => Ok((isolated, detail)),
        other => Err(format!("{other:?}")),
    }
}

/// The model, run in the worker.
pub struct Isolated {
    path: PathBuf,
    ctx: u32,
    threads: i32,
    gpu: bool,
    context: usize,
    cancel: Arc<AtomicBool>,
    inner: Mutex<Inner>,
    pub name: String,
}

impl Isolated {
    /// Starts the worker, checks it can't reach the network, and loads the model in it.
    pub fn load(path: &Path, ctx: u32, threads: i32, gpu: bool, cancel: Arc<AtomicBool>) -> Result<Self, String> {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut me = Self { path: path.to_path_buf(), ctx, threads, gpu, context: ctx as usize, cancel, inner: Mutex::new(Inner { worker: None }), name };
        me.context = me.ensure()?;
        Ok(me)
    }

    /// A worker with the model loaded, started if there isn't one; the model's context.
    fn ensure(&self) -> Result<usize, String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.worker.is_some() {
            return Ok(self.context);
        }
        sandbox::allow(&self.path)?;
        inner.worker = Some(Worker::start()?);
        match self.ask_locked(&mut inner, &Request::Probe)? {
            Reply::Probe { isolated: true, .. } => {}
            Reply::Probe { detail, .. } => {
                inner.worker = None;
                return Err(format!("Analyses run only where the model can't reach the internet, and that couldn't be set up here ({detail})."));
            }
            other => return Err(format!("The analysis process answered oddly: {other:?}")),
        }
        let load = Request::Load { path: self.path.to_string_lossy().into_owned(), ctx: self.ctx, threads: self.threads, gpu: self.gpu };
        match self.ask_locked(&mut inner, &load)? {
            Reply::Loaded { context } => Ok(context),
            Reply::Error { message } => {
                inner.worker = None;
                Err(message)
            }
            other => Err(format!("The analysis process answered oddly: {other:?}")),
        }
    }

    /// Sends a request and waits for the answer, ending the worker if the analysis is cancelled or the
    /// worker stops.
    fn ask_locked(&self, inner: &mut Inner, request: &Request) -> Result<Reply, String> {
        self.ask_streaming(inner, request, &mut |_| {})
    }

    /// As `ask_locked`, with each `Text` piece that comes before the answer given to `on_text`.
    fn ask_streaming(&self, inner: &mut Inner, request: &Request, on_text: &mut dyn FnMut(&str)) -> Result<Reply, String> {
        let worker = inner.worker.as_mut().ok_or("The analysis process isn't running.")?;
        let line = serde_json::to_string(request).map_err(|e| e.to_string())?;
        let sent = writeln!(worker.input, "{line}").and_then(|_| worker.input.flush());
        if sent.is_err() {
            inner.worker = None;
            return Err("The analysis process stopped. Try again.".into());
        }
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                inner.worker = None;
                return Err(CANCELLED.into());
            }
            match worker.lines.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => match serde_json::from_str(&line).map_err(|e| format!("The analysis process answered oddly: {e}"))? {
                    Reply::Text { text } => on_text(&text),
                    reply => return Ok(reply),
                },
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    inner.worker = None;
                    return Err("The analysis process stopped unexpectedly, perhaps out of memory. Try again, or a smaller model.".into());
                }
            }
        }
    }

    fn ask(&self, request: &Request) -> Result<Reply, String> {
        self.ensure()?;
        let mut inner = self.inner.lock().unwrap();
        self.ask_locked(&mut inner, request)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.process.kill();
    }
}

impl Model for Isolated {
    fn complete(&mut self, system: &str, user: &str, grammar: &str, _schema: &serde_json::Value, max_tokens: usize) -> Result<String, String> {
        let request = Request::Complete { system: system.into(), user: user.into(), grammar: grammar.into(), max_tokens, stream: false };
        match self.ask(&request)? {
            Reply::Done { text } => Ok(text),
            Reply::Error { message } => Err(message),
            other => Err(format!("The analysis process answered oddly: {other:?}")),
        }
    }

    fn stream(&mut self, system: &str, user: &str, grammar: &str, _schema: &serde_json::Value, max_tokens: usize, on_text: &mut dyn FnMut(&str)) -> Result<String, String> {
        let request = Request::Complete { system: system.into(), user: user.into(), grammar: grammar.into(), max_tokens, stream: true };
        self.ensure()?;
        let mut inner = self.inner.lock().unwrap();
        match self.ask_streaming(&mut inner, &request, on_text)? {
            Reply::Done { text } => Ok(text),
            Reply::Error { message } => Err(message),
            other => Err(format!("The analysis process answered oddly: {other:?}")),
        }
    }

    fn count(&self, text: &str) -> usize {
        match self.ask(&Request::Count { text: text.into() }) {
            Ok(Reply::Count { n }) => n,
            // A safe overestimate if the worker can't say.
            _ => text.len() / 2 + 1,
        }
    }

    fn context(&self) -> usize {
        self.context
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

#[cfg(windows)]
mod sandbox {
    //! An AppContainer with no capabilities, given read access to the program and the model only.
    use super::*;
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};
    use windows_sys::Win32::Security::Authorization::{
        GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT,
        TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::Isolation::{CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName};
    use windows_sys::Win32::Security::{FreeSid, ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE, PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES};
    use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute,
        CREATE_NO_WINDOW, EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };

    const CONTAINER: &str = "SuperteemVoice.Analysis";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The container's identity, made the first time.
    struct Sid(PSID);

    impl Drop for Sid {
        fn drop(&mut self) {
            unsafe { FreeSid(self.0) };
        }
    }

    fn container() -> Result<Sid, String> {
        let name = wide(CONTAINER);
        let mut sid: PSID = null_mut();
        let made = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                wide("Voice by Superteem analysis").as_ptr(),
                wide("Runs the language model for analyses, without network access.").as_ptr(),
                null(),
                0,
                &mut sid,
            )
        };
        if made < 0 {
            // Already made: its identity comes from its name.
            let found = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
            if found < 0 {
                return Err(format!("Couldn't set up the sandbox for analyses (error {made:#x})."));
            }
        }
        Ok(Sid(sid))
    }

    /// Lets the container read and run `path`, a file.
    fn grant(path: &Path, sid: &Sid) -> Result<(), String> {
        let name = wide(&path.to_string_lossy());
        unsafe {
            let mut dacl: *mut ACL = null_mut();
            let mut descriptor = null_mut();
            let got = GetNamedSecurityInfoW(name.as_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), &mut dacl, null_mut(), &mut descriptor);
            if got != 0 {
                return Err(format!("Couldn't read the permissions of {} (error {got}).", path.display()));
            }
            let access = EXPLICIT_ACCESS_W {
                grfAccessPermissions: FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
                grfAccessMode: GRANT_ACCESS,
                grfInheritance: NO_INHERITANCE,
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: null_mut(),
                    MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                    ptstrName: sid.0 as *mut u16,
                },
            };
            let mut updated: *mut ACL = null_mut();
            let merged = SetEntriesInAclW(1, &access, dacl, &mut updated);
            let set = if merged == 0 {
                SetNamedSecurityInfoW(name.as_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), updated, null())
            } else {
                merged
            };
            if !updated.is_null() {
                LocalFree(updated as _);
            }
            LocalFree(descriptor as _);
            if set != 0 {
                return Err(format!("Couldn't let the analysis sandbox read {} (error {set}).", path.display()));
            }
        }
        Ok(())
    }

    /// Lets the sandbox read the model and the program it runs.
    pub fn allow(model: &Path) -> Result<(), String> {
        let sid = container()?;
        grant(model, &sid)?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        // An installation for everyone is in Program Files, which sandboxes can already read, and which
        // this app can't change: that's fine.
        if let Err(e) = grant(&exe, &sid) {
            if !exe.to_string_lossy().to_lowercase().contains("program files") {
                return Err(e);
            }
        }
        Ok(())
    }

    pub struct Process {
        process: usize,
        job: usize,
    }

    impl Process {
        pub fn kill(&mut self) {
            unsafe {
                TerminateProcess(self.process as HANDLE, 1);
                CloseHandle(self.process as HANDLE);
                CloseHandle(self.job as HANDLE);
            }
            self.process = 0;
            self.job = 0;
        }
    }

    type Pipes = (Box<dyn Write + Send>, Box<dyn Read + Send>, Process);

    pub fn spawn() -> Result<Pipes, String> {
        let sid = container()?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        unsafe {
            let inherit = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: null_mut(), bInheritHandle: 1 };
            let (mut child_in, mut to_child, mut from_child, mut child_out): (HANDLE, HANDLE, HANDLE, HANDLE) = (null_mut(), null_mut(), null_mut(), null_mut());
            if CreatePipe(&mut child_in, &mut to_child, &inherit, 0) == 0 || CreatePipe(&mut from_child, &mut child_out, &inherit, 0) == 0 {
                return Err("Couldn't make the pipes to the analysis process.".into());
            }
            // Only the child's ends are passed on, and only those two handles.
            SetHandleInformation(to_child, HANDLE_FLAG_INHERIT, 0);
            SetHandleInformation(from_child, HANDLE_FLAG_INHERIT, 0);
            let mut size = 0usize;
            InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut size);
            let mut list = vec![0u8; size];
            let attributes = list.as_mut_ptr() as _;
            if InitializeProcThreadAttributeList(attributes, 2, 0, &mut size) == 0 {
                return Err("Couldn't prepare the analysis process.".into());
            }
            let capabilities = SECURITY_CAPABILITIES { AppContainerSid: sid.0, Capabilities: null_mut(), CapabilityCount: 0, Reserved: 0 };
            let handles = [child_in, child_out];
            let ok = UpdateProcThreadAttribute(
                attributes,
                0,
                PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                &capabilities as *const _ as _,
                std::mem::size_of::<SECURITY_CAPABILITIES>(),
                null_mut(),
                null(),
            ) != 0
                && UpdateProcThreadAttribute(attributes, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, handles.as_ptr() as _, std::mem::size_of_val(&handles), null_mut(), null())
                    != 0;
            if !ok {
                DeleteProcThreadAttributeList(attributes);
                return Err("Couldn't prepare the analysis sandbox.".into());
            }
            let mut startup: STARTUPINFOEXW = std::mem::zeroed();
            startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdInput = child_in;
            startup.StartupInfo.hStdOutput = child_out;
            startup.StartupInfo.hStdError = null_mut();
            startup.lpAttributeList = attributes;
            let program = wide(&exe.to_string_lossy());
            let mut command = wide(&format!("\"{}\" {ARG}", exe.display()));
            let mut info: PROCESS_INFORMATION = std::mem::zeroed();
            let started = CreateProcessW(
                program.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
                null(),
                null(),
                &startup.StartupInfo,
                &mut info,
            );
            DeleteProcThreadAttributeList(attributes);
            CloseHandle(child_in);
            CloseHandle(child_out);
            if started == 0 {
                CloseHandle(to_child);
                CloseHandle(from_child);
                return Err(format!("Couldn't start the analysis process (error {}).", std::io::Error::last_os_error()));
            }
            CloseHandle(info.hThread);
            // The worker ends with the app, however the app ends.
            let job = CreateJobObjectW(null(), null());
            if !job.is_null() {
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits as *const _ as _, std::mem::size_of_val(&limits) as u32);
                AssignProcessToJobObject(job, info.hProcess);
            }
            let input = std::fs::File::from_raw_handle(to_child as RawHandle);
            let output = std::fs::File::from_raw_handle(from_child as RawHandle);
            Ok((Box::new(input), Box::new(output), Process { process: info.hProcess as usize, job: job as usize }))
        }
    }
}

#[cfg(target_os = "macos")]
mod sandbox {
    //! The system sandbox, with every kind of networking denied, name lookups included.
    use super::*;
    use std::process::{Command, Stdio};

    const PROFILE: &str = r#"(version 1)
(allow default)
(deny network*)
(deny mach-lookup (global-name "com.apple.dnssd.service") (global-name "com.apple.mDNSResponder") (global-name "com.apple.networkd") (global-name "com.apple.nehelper"))"#;

    pub fn allow(_model: &Path) -> Result<(), String> {
        Ok(())
    }

    pub struct Process(std::process::Child);

    impl Process {
        pub fn kill(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    type Pipes = (Box<dyn Write + Send>, Box<dyn Read + Send>, Process);

    pub fn spawn() -> Result<Pipes, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut child = Command::new("/usr/bin/sandbox-exec")
            .arg("-p")
            .arg(PROFILE)
            .arg(&exe)
            .arg(ARG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Couldn't start the analysis sandbox: {e}"))?;
        let input = child.stdin.take().ok_or("No pipe to the analysis process.")?;
        let output = child.stdout.take().ok_or("No pipe from the analysis process.")?;
        Ok((Box::new(input), Box::new(output), Process(child)))
    }
}
