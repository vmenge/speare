
TBD 🏗️

<p hidden>
process lifecycle
- process spawns and lives in memory forever until its stopped
- stopped by its parent being stopped, by being stopped manually or by its parent supervision strategy
- when a process is spawned, the init method is called to create the process instance
- when a process is stopped the exit method is called. 
- when a process is restarted, the exit method is first called, and then the init to create a new instance of the process. the handle remains the same though.
</p>