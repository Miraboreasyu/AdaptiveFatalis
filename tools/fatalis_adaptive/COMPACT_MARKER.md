# Experimental compact marker

This is an opt-in prototype, not a replacement for the tested v0.6.1 release.

Place an empty `compact-marker.enabled` file in the plugin private directory beside `config.json`. Strategy selection stays unchanged. On startup the worker attempts to install a guarded interpreter hook **before compiling/deploying compact bytecode**. If detection/installation fails it compiles the original native wrappers instead. Remove the opt-in file and restart to use native mode.

The compiler preserves the exact table/exit packed IDs and original Global calls. Instead of native digit clears/increments plus counter increment it emits one private marker opcode (`0x7f414446`, parameter1 packed ID, parameter2 `0x314d4641`). Each wrapper contains marker, original Global call, return, node-end. The handler publishes the same five decimal registers and counter; learning scripts and historical event/profile formats do not change.

A shared host lock serializes the compact handler's register write and compact sampler's read. It does not protect against arbitrary other plugins or game code modifying the reserved registers. Unmodified interpreter calls go through the trampoline, outside this lock.

The locator requires a unique full function prologue plus known interior signature and a matching PE runtime-function entry, rather than treating the interior signature as a function entry or using historical fixed virtual addresses. Unknown builds and detected extender conflicts must fail closed. The plugin stays pinned for process lifetime; the hook is not removed on worker shutdown. The normal deployment ownership/watchdog restoration still applies.

Important: an unexpected crash, a separately restored old plugin, or killing recovery helpers can leave compact THK on disk. Restore the managed baseline/owned override before removing this experimental DLL. Do not redistribute compact generated THK as a standalone mod: it requires the matching handler active before the resource executes.

No game installation or live hook execution has been performed by the implementation process. Offline parser/structural tests cannot establish that a live hook is safe or that all versions are compatible. Keep backups and test without unrelated THK extenders. A resource-size reduction does not establish a measurable FPS improvement on low-end PCs.
