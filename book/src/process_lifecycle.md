
TBD 🏗️

<p hidden>
actor lifecycle
- actor spawns and lives in memory forever until its stopped
- stopped by its parent being stopped, by being stopped manually or by its parent supervision strategy
- when a actor is spawned, the init method is called to create the actor instance
- when a actor is stopped the exit method is called. 
- when a actor is restarted, the exit method is first called, and then the init to create a new instance of the actor. the handle remains the same though.
</p>