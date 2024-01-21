TBD 🏗️

<p hidden>
supervisoin
- top level processes created by a node are unsupervised
- every process created by another process is superivsed by its parent
- supervision means that the parent decide can decide what to do with its children when one of them errors.
- two main strategies, one for one, one for all
- can restart, stop, resume or escalate
- a process will finish handling whatever message it is  currently handling before stopping (or restarting) itself
</p>