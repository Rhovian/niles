import { useEffect, useRef, useState } from 'preact/hooks';
import { type Connection, type Message, type Session, type Turn, RECENT_TURN_LIMIT, sendMessage, sessionSchema, snapshotSchema, turnSchema, upsertTurn } from './api';
import { Markdown } from './Markdown';

const HISTORY_NOTICE_MS = 5_000;

type Outgoing = Message & { id: string };
export function Chat({ session, onStatus }: { session: Session; onStatus: (session: Session) => void }) {
  const [history, setHistory] = useState<{ turns: Turn[]; truncated: boolean }>({ turns: [], truncated: false });
  const [historyNoticeDismissed, setHistoryNoticeDismissed] = useState(false);
  const [outgoing, setOutgoing] = useState<Outgoing[]>([]);
  const [connection, setConnection] = useState<Connection>('catching up');
  const [streamError, setStreamError] = useState('');
  const [sendError, setSendError] = useState('');
  const [draft, setDraft] = useState('');
  const [attempt, setAttempt] = useState(0);
  const bottom = useRef<HTMLDivElement>(null);
  useEffect(() => {
    setConnection('catching up'); setStreamError('');
    const stream = new EventSource(`/api/sessions/${encodeURIComponent(session.id)}/events`);
    const receive = (name: string, accept: (data: unknown) => void) => {
      stream.addEventListener(name, event => {
        try { accept(JSON.parse((event as MessageEvent<string>).data)); }
        catch (cause) {
          stream.close(); setConnection('disconnected');
          setStreamError(`Invalid event from API: ${String(cause)}`);
        }
      });
    };
    // A snapshot, rather than open, proves catch-up completed.
    stream.onopen = () => setConnection('catching up');
    stream.onerror = () => {
      if (stream.readyState === EventSource.CLOSED) {
        setConnection('disconnected');
        setStreamError('Session stream closed. Check access permissions, session availability, and API compatibility, then reconnect.');
      } else setConnection('reconnecting');
    };
    receive('snapshot', data => { setHistory(snapshotSchema.parse(data)); setConnection('live'); setSendError(''); });
    receive('turn', data => {
      const turn = turnSchema.parse(data);
      setHistory(old => {
        const turns = upsertTurn(old.turns, turn);
        return { turns: turns.slice(-RECENT_TURN_LIMIT), truncated: old.truncated || turns.length > RECENT_TURN_LIMIT };
      });
    });
    receive('status', data => {
      const status = sessionSchema.parse(data);
      if (status.id !== session.id) throw new Error('Status session does not match stream');
      onStatus(status);
    });
    return () => stream.close();
  }, [session.id, onStatus, attempt]);
  useEffect(() => {
    if (!history.truncated) return;
    const timer = setTimeout(() => setHistoryNoticeDismissed(true), HISTORY_NOTICE_MS);
    return () => clearTimeout(timer);
  }, [history.truncated]);
  useEffect(() => { bottom.current?.scrollIntoView({ block: 'end' }); }, [history, outgoing]);
  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!draft.trim() || connection !== 'live') return;
    const message: Outgoing = { id: crypto.randomUUID(), text: draft };
    setDraft(''); setSendError('');
    setOutgoing(old => [...old, message]);
    try {
      await sendMessage(session.id, { text: message.text });
      setSendError('');
    } catch (cause) {
      // Preserve anything typed while this request was in flight.
      setDraft(current => current ? `${current}\n${message.text}` : message.text);
      setSendError(`Send was not confirmed. Draft restored; check the transcript before sending again. ${String(cause)}`);
    } finally { setOutgoing(old => old.filter(item => item.id !== message.id)); }
  }
  return <section class="chat" aria-label={`Chat with ${session.project}`}>
    <header><div><h2>{session.project}</h2><small>{session.agent.family} · {session.agent.entry_skill}</small></div>
      <span class={`pill ${connection === 'live' ? 'live' : ''}`} role="status">{connection}</span></header>
    {connection === 'reconnecting' && <p class="banner" role="alert">Cannot reach Niles. Check that you’re connected to your tailnet.</p>}
    {streamError && <p class="banner" role="alert">{streamError} <button onClick={() => setAttempt(old => old + 1)}>Reconnect</button></p>}
    {sendError && <p class="banner" role="alert">{sendError}</p>}
    {history.truncated && !historyNoticeDismissed && <p class="banner">Showing the most recent {RECENT_TURN_LIMIT} turns. Earlier history is available at the desk.</p>}
    <div class="transcript" aria-label="Transcript" aria-live="polite">
      {history.turns.map(turn => <article class={`bubble ${turn.role}`} key={turn.id}>
        <small>{turn.role === 'agent' ? 'Agent' : 'You'}</small><Markdown text={turn.text} />
      </article>)}
      {outgoing.map(item => <article class="bubble user" key={item.id}>
        <small>You · Sending…</small><p class="plaintext">{item.text}</p>
      </article>)}
      <div ref={bottom} />
    </div>
    <form onSubmit={submit}>
      <label for="message">Message agent</label>
      <div class="composer"><textarea id="message" rows={2} value={draft} onInput={event => setDraft(event.currentTarget.value)} />
        <button disabled={connection !== 'live' || !draft.trim()} type="submit">Send</button></div>
    </form>
  </section>;
}
