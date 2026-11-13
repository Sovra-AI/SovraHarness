# Schedules

Use **Schedules** to run a prompt once or repeatedly on this computer. Sovra must
be running and the computer awake; this is not a hosted scheduler or an OS wake-up
service. Prompt runs use the configured chat model and may incur provider costs.

## Create and manage a task

1. Open **Schedules → Create schedule**.
2. Give it a name, a cadence in ordinary words, and the prompt to run.
3. Choose **Save schedule**, then check the next-run time in the list.

The form stays open while saving. If saving fails, your entries remain available
to retry. **Hide form** keeps the unfinished task for this session; **Continue
task** reopens it. After a confirmed save, the next new task starts with empty
fields. A blank timing field means every day at 9:00.

Examples of supported cadence text are `in 1 minute`, `just once`,
`every weekday at 9`, `Fridays at 4`, and `every 4 hours`. You can also ask the
schedule helper in chat to create or manage a task. Check the saved task rather
than treating a conversational promise as confirmation.

The page shows active and paused tasks, recent results and a timeline. Use
**Run in chat**, **Pause**, **Resume**, or **Delete** on a task. Pause prevents future
scheduled starts; it does not cancel work already running. One-shot tasks pause
after firing. Resuming one schedules its delay again.

If saving a change fails, the previously saved task stays in effect. A new task
does not become active, and a requested run does not start, until the scheduler
has saved the change. Automatic runs retry after 30 seconds when storage is
unavailable; other controls remain available during that wait.

## Timing and results

Due tasks run once when the scheduler next wakes. Repeating tasks calculate their
next time from that wake, rather than replaying every interval missed while the
app was closed. If the same task is still running, the overlapping occurrence is
recorded as **Skipped**. On restart, unfinished runs are marked failed.

Each task stores a fixed UTC offset captured when it is created. The runtime
reads the computer's offset at startup (falling back to UTC if unavailable).
Schedules do not track named time zones or automatically adjust saved offsets
for daylight saving or travel. Recreate time-sensitive tasks after checking the
new offset. The run history retains at most 200 entries across the schedule book.

## Timed messages

The chat helper can create a confirmed WhatsApp or Telegram text with the app,
recipient, exact wording and time. Creating this job authorizes the later send;
there is no second **Send** review when it fires. Use a concrete recipient, such
as an international number, exact chat/group ID or a selected saved person.
A first name alone is refused. The channel must be connected when the job runs.
See [Messaging](communications.md) for recipient and delivery limitations.

A confirmed web search can be saved the same way for one WhatsApp group. The
words to search for are stored; the short note is written when the job fires,
from that search, and sent to the one group named when you confirmed. A unique
group name is stored as that group's id. Nothing is sent when the job is saved.
If the name matches more than one group, or none, the job is refused in plain
words and is not scheduled. `every morning at 6:15 am` and `every morning at 615 am`
both mean every day at 06:15 in the task's saved UTC offset. At offset +240
(Dubai / UAE), 06:15 local is 02:15 UTC. The app must be running and WhatsApp
connected when the job fires. A disconnected WhatsApp records a failed run and
sends nothing. A second fire while that run is still going is recorded as
**Skipped** and does not send again.

A failed send keeps its reason on the run. Inspect the result before scheduling
another message; service acceptance does not establish delivery or reading.

## Storage and implementation

Tasks and run history live in `schedules.json` under the [app data directory](configuration.md).
The file contains prompt and timed-message text. Writes use a temporary file and
rename. A save error is reported, but the current runtime may still execute the
in-memory task; persistence is not a transactional dispatch guarantee.

- [Core rules](../crates/sovra-core/src/schedules.rs): cadence parsing, due times,
  overlap handling, bounded history and reducers.
- [Runtime](../crates/sovra-runtime/src/schedules.rs): persistence, wake loop and dispatch.
- [Native page](../crates/sovra-ui/src/schedules_view.rs): task form, list and timeline.
