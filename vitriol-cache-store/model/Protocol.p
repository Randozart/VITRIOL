// One engine owner serializes publication, lease changes and reclamation.
event eRead: machine;
event eReadGranted: bool;
event eRelease: machine;
event eOldCommit: machine;
event eNewDurable: machine;
event eAck;
event eDone;
event eDeleted: int;
event eVisible;

machine CacheOwner {
    var readers: int;
    var frontier: int;
    var oldVisible: bool;
    var fence: bool;
    start state Ready {
        entry (enabled: bool) { fence = enabled; frontier = 2; oldVisible = true; }
        on eRead do (client: machine) {
            if (oldVisible) { readers = readers + 1; }
            send client, eReadGranted, oldVisible;
        }
        on eRelease do (client: machine) {
            assert readers > 0, "release without lease";
            readers = readers - 1;
            Collect();
            send client, eAck;
        }
        on eNewDurable do (client: machine) {
            // New bytes and the generation acknowledgment are durable before this transition.
            frontier = 3;
            Collect();
            send client, eAck;
        }
        on eOldCommit do (client: machine) {
            if (!fence || frontier <= 1) { oldVisible = true; announce eVisible; }
            send client, eAck;
        }
    }
    fun Collect() {
        // Two newer durable anchors exist; the old generation has no other pins here.
        if (frontier >= 3 && readers == 0 && oldVisible) {
            oldVisible = false;
            announce eDeleted, readers;
        }
    }
}

machine Reader {
    start state Run {
        entry (args: (owner: machine, driver: machine)) {
            var granted: bool;
            send args.owner, eRead, this;
            receive { case eReadGranted: (value: bool) { granted = value; } }
            if (granted) {
                send args.owner, eRelease, this;
                receive { case eAck: {} }
            }
            send args.driver, eDone;
            raise halt;
        }
    }
}

machine Writer {
    start state Run {
        entry (args: (owner: machine, driver: machine, stale: bool)) {
            if (args.stale) { send args.owner, eOldCommit, this; }
            else { send args.owner, eNewDurable, this; }
            receive { case eAck: {} }
            send args.driver, eDone;
            raise halt;
        }
    }
}

spec RetirementContract observes eDeleted, eVisible {
    var retired: bool;
    start state Watching {
        on eDeleted do (readers: int) {
            assert readers == 0, "deleted snapshot with active reader";
            retired = true;
        }
        on eVisible do {
            assert !retired, "late writer resurrected retired generation";
        }
    }
}

fun Run(driver: machine, fence: bool) {
    var owner: machine;
    owner = new CacheOwner(fence);
    new Reader((owner = owner, driver = driver));
    new Writer((owner = owner, driver = driver, stale = true));
    new Writer((owner = owner, driver = driver, stale = false));
    receive { case eDone: {} }
    receive { case eDone: {} }
    receive { case eDone: {} }
}
machine Serialized { start state Init { entry { Run(this, true); } } }
machine MissingFence { start state Init { entry { Run(this, false); } } }

test tcSerialized [main=Serialized]: assert RetirementContract in {Serialized, CacheOwner, Reader, Writer};
test tcMissingFence [main=MissingFence]: assert RetirementContract in {MissingFence, CacheOwner, Reader, Writer};
