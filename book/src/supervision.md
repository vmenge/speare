TBD 🏗️

<p hidden>
supervisoin
- top level actors created by a node are unsupervised
- every actor created by another actor is superivsed by its parent
- supervision means that the parent decide can decide what to do with its children when one of them errors.
- two main strategies, one for one, one for all
- can restart, stop, resume or escalate
- a actor will finish handling whatever message it is  currently handling before stopping (or restarting) itself
</p>