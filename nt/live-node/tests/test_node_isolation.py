"""Todo 41: one live node per account, enforced rather than assumed.

Named `test_node_isolation` rather than `test_isolation` because pytest
derives a module name from the BASENAME when a test directory has no
`__init__.py`, and `nt/backtest-worker/tests/test_isolation.py` already claims
that name. The collision does not appear when either suite runs on its own --
only when the whole `nt` tree is collected, which is what CI does.

Two processes trading one account double every order either places, and
neither can see the other's in-flight state. Migration 0016 stops the API
recording two nodes; nothing there stops someone running the process twice on
a host, which is what this closes.
"""
from __future__ import annotations

import multiprocessing as mp
import os
from queue import Empty

import pytest

from live_node.isolation import AccountLock, NodeAlreadyRunning

ACCOUNT = "acct-live-1"


def _stale_reclaim_worker(
    lock_dir: str,
    account: str,
    rounds: int,
    start_barrier,
    finish_barrier,
    done_barrier,
    results,
) -> None:
    """Race one account lock repeatedly from independent processes."""

    for round_number in range(rounds):
        start_barrier.wait(timeout=20)
        lock = AccountLock(lock_dir, account)
        acquired = False
        try:
            holder = lock.acquire()
        except NodeAlreadyRunning as exc:
            results.put((round_number, "refused", os.getpid(), exc.holder.pid))
        else:
            acquired = True
            results.put((round_number, "acquired", os.getpid(), holder.pid))

        # Keep the winner alive until every contender has observed the result;
        # otherwise a fast worker exit would leave a stale lock for the next
        # round and make the test exercise process cleanup instead of mutual
        # exclusion.
        finish_barrier.wait(timeout=20)
        if acquired:
            lock.release()
        done_barrier.wait(timeout=20)


def _live_holder_worker(lock_dir: str, account: str, started, release, results) -> None:
    lock = AccountLock(lock_dir, account)
    holder = lock.acquire()
    results.put(holder.pid)
    started.set()
    release.wait(timeout=20)
    lock.release()


def test_a_second_node_for_the_same_account_is_refused(tmp_path):
    first = AccountLock(tmp_path, ACCOUNT)
    held = first.acquire()
    assert held.pid == os.getpid()

    second = AccountLock(tmp_path, ACCOUNT)
    with pytest.raises(NodeAlreadyRunning) as excinfo:
        second.acquire()
    # The refusal names who holds it, so an operator can check whether that
    # process is really the one they think it is.
    assert excinfo.value.holder.pid == os.getpid()
    assert ACCOUNT in str(excinfo.value)


def test_a_repeated_acquire_does_not_lose_the_original_ownership(tmp_path):
    lock = AccountLock(tmp_path, ACCOUNT)
    lock.acquire()
    with pytest.raises(NodeAlreadyRunning):
        lock.acquire()
    lock.release()
    assert not lock.path.exists()


def test_different_accounts_do_not_block_each_other(tmp_path):
    # One node per ACCOUNT, not one node per host: the Owner may run several.
    a = AccountLock(tmp_path, "acct-a")
    b = AccountLock(tmp_path, "acct-b")
    a.acquire()
    b.acquire()
    assert a.path != b.path


def test_releasing_lets_the_next_node_start(tmp_path):
    first = AccountLock(tmp_path, ACCOUNT)
    first.acquire()
    first.release()

    second = AccountLock(tmp_path, ACCOUNT)
    second.acquire()  # must not raise
    assert second.read_holder().pid == os.getpid()


def test_a_lock_left_by_a_dead_process_is_reclaimed(tmp_path):
    # A crash must not need a human to delete a file before Live can come
    # back: "just remove the lock file" becomes routine, and then the
    # exclusivity stops meaning anything the one time it matters.
    lock_path = tmp_path / f"live-node-{ACCOUNT}.lock"
    lock_path.write_text("999999999\n" + ACCOUNT, encoding="utf-8")

    reclaimed = AccountLock(tmp_path, ACCOUNT)
    info = reclaimed.acquire()
    assert info.pid == os.getpid()


def test_a_lock_held_by_a_live_process_is_never_reclaimed(tmp_path):
    # The complement of the test above, and the one that matters: reclaiming
    # is safe only because it is verified against the OS. A rule based on the
    # file's age would eventually steal a lock from a running node.
    lock_path = tmp_path / f"live-node-{ACCOUNT}.lock"
    lock_path.write_text(f"{os.getpid()}\n{ACCOUNT}", encoding="utf-8")

    with pytest.raises(NodeAlreadyRunning):
        AccountLock(tmp_path, ACCOUNT).acquire()


def test_concurrent_stale_reclamation_has_one_winner_every_time(tmp_path):
    """A dead holder is reclaimed by exactly one synchronized starter."""

    context = mp.get_context("spawn")
    workers = 6
    rounds = 12
    start_barrier = context.Barrier(workers + 1)
    finish_barrier = context.Barrier(workers + 1)
    done_barrier = context.Barrier(workers + 1)
    results = context.Queue()
    processes = [
        context.Process(
            target=_stale_reclaim_worker,
            args=(
                str(tmp_path),
                ACCOUNT,
                rounds,
                start_barrier,
                finish_barrier,
                done_barrier,
                results,
            ),
        )
        for _ in range(workers)
    ]
    lock_path = tmp_path / f"live-node-{ACCOUNT}.lock"

    for process in processes:
        process.start()
    completed = False
    try:
        for round_number in range(rounds):
            # This PID is intentionally outside the process table. Every
            # starter therefore has the same stale holder to reclaim.
            lock_path.write_text(f"999999999\n{ACCOUNT}", encoding="utf-8")
            start_barrier.wait(timeout=20)
            finish_barrier.wait(timeout=20)
            observed = []
            for _ in range(workers):
                try:
                    observed.append(results.get(timeout=20))
                except Empty:
                    pytest.fail("a stale-lock contender did not report")
            assert [item[0] for item in observed] == [round_number] * workers
            assert sum(item[1] == "acquired" for item in observed) == 1
            assert sum(item[1] == "refused" for item in observed) == workers - 1
            done_barrier.wait(timeout=20)
            assert not lock_path.exists()
        completed = True
    finally:
        if not completed:
            for barrier in (start_barrier, finish_barrier, done_barrier):
                barrier.abort()
        for process in processes:
            process.join(timeout=20)
        for process in processes:
            if process.is_alive():
                process.terminate()
                process.join(timeout=20)

    assert all(process.exitcode == 0 for process in processes)


def test_a_live_holder_cannot_be_stolen_by_another_process(tmp_path):
    context = mp.get_context("spawn")
    started = context.Event()
    release = context.Event()
    results = context.Queue()
    process = context.Process(
        target=_live_holder_worker,
        args=(str(tmp_path), ACCOUNT, started, release, results),
    )
    process.start()
    try:
        assert started.wait(timeout=20)
        assert results.get(timeout=20) == process.pid
        with pytest.raises(NodeAlreadyRunning) as excinfo:
            AccountLock(tmp_path, ACCOUNT).acquire()
        assert excinfo.value.holder.pid == process.pid
        assert (tmp_path / f"live-node-{ACCOUNT}.lock").exists()
    finally:
        release.set()
        process.join(timeout=20)
        if process.is_alive():
            process.terminate()
            process.join(timeout=20)
    assert process.exitcode == 0
    assert not (tmp_path / f"live-node-{ACCOUNT}.lock").exists()


def test_a_corrupt_lock_refuses_rather_than_clearing_itself(tmp_path):
    # An unreadable lock is a fault someone should look at, not something to
    # silently clear -- clearing it is indistinguishable from stealing a live
    # node's claim.
    lock_path = tmp_path / f"live-node-{ACCOUNT}.lock"
    lock_path.write_text("not-a-pid\n" + ACCOUNT, encoding="utf-8")

    with pytest.raises(NodeAlreadyRunning):
        AccountLock(tmp_path, ACCOUNT).acquire()


def test_release_does_not_delete_a_successors_claim(tmp_path):
    # A late release from a process that already lost the lock must not remove
    # the new owner's file, even if a same-process test double has the same
    # PID in its replacement payload. The inode captured at acquire is the
    # ownership proof that distinguishes the claims.
    first = AccountLock(tmp_path, ACCOUNT)
    first.acquire()
    # Simulate a successor taking over after this process was presumed dead.
    first.path.write_text(f"{os.getpid()}\n{ACCOUNT}", encoding="utf-8")
    first.release()
    assert first.path.exists(), "a foreign claim must survive our release"


def test_the_lock_works_as_a_context_manager(tmp_path):
    with AccountLock(tmp_path, ACCOUNT) as info:
        assert info.pid == os.getpid()
    # Released on exit, so the next node starts cleanly.
    AccountLock(tmp_path, ACCOUNT).acquire()


def test_an_account_lock_requires_an_account(tmp_path):
    with pytest.raises(ValueError):
        AccountLock(tmp_path, "")


def test_an_account_id_cannot_escape_the_lock_directory(tmp_path):
    with pytest.raises(ValueError):
        AccountLock(tmp_path, "../outside")


def test_a_symlinked_lock_directory_is_rejected(tmp_path):
    target = tmp_path / "target"
    target.mkdir()
    link = tmp_path / "link"
    try:
        link.symlink_to(target, target_is_directory=True)
    except (NotImplementedError, OSError):
        pytest.skip("symlinks unavailable")
    with pytest.raises(ValueError):
        AccountLock(link, ACCOUNT)


def test_liveness_recognises_a_dead_pid_on_this_platform():
    # Regression guard for a real defect. The first version used
    # `os.kill(pid, 0)`, which is correct on POSIX and wrong on Windows:
    # measured here, a nonexistent pid raises OSError WinError 87 -- not
    # ProcessLookupError, and with an errno that is not ESRCH -- so the
    # reasonable-looking `except OSError: return True` fallback reported a
    # long-dead process as alive. The effect was that a stale lock was never
    # reclaimed and Live could not restart after a crash without someone
    # deleting a file by hand.
    #
    # Asserted directly rather than only through the lock, because the lock
    # test above passes for the wrong reason if liveness always returns False.
    from live_node.isolation import _pid_alive

    assert _pid_alive(os.getpid()) is True, "this very process is alive"
    assert _pid_alive(999_999_999) is False, "a pid this high does not exist"
    # Nonsense pids are dead, not alive: they must never keep a lock held.
    assert _pid_alive(0) is False
    assert _pid_alive(-1) is False


def test_an_unreadable_lock_is_not_the_same_as_an_unheld_one():
    # The distinction the pid=None value exists to preserve. Conflating them
    # is how a corrupt lock silently becomes a stolen one.
    from live_node.isolation import LockInfo

    unheld = None
    unreadable = LockInfo(pid=None, account_id=ACCOUNT)
    held = LockInfo(pid=os.getpid(), account_id=ACCOUNT)

    assert unheld is None
    assert unreadable.is_readable is False
    assert held.is_readable is True
    # The refusal message says which situation it is, because "already served
    # by pid None" would send an operator looking for a process.
    assert "unreadable" in str(NodeAlreadyRunning(unreadable))
    assert str(os.getpid()) in str(NodeAlreadyRunning(held))
