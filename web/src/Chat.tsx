import { useEffect, useLayoutEffect, useRef, useState } from 'preact/hooks';
import { type Connection, type Session, type Turn, RECENT_TURN_LIMIT, sendMessage, sessionSchema, snapshotSchema, turnSchema } from './api';
import { Markdown } from './Markdown';

const HISTORY_NOTICE_MS = 5_000;

type ChatTurn = Turn & { local?: { id: string; sent: boolean } };

// Preact reconciles keyed children; transcript echoes must retain the local key and position.
// https://github.com/preactjs/preact/blob/11.0.1/src/diff/children.js
function mergeTurns(current: ChatTurn[], incoming: Turn[]): ChatTurn[] {
  return incoming.reduce((turns, turn) => {
    const existing = turns.findIndex(item => item.id === turn.id);
    const index = existing >= 0 ? existing : turn.role === 'user'
      ? turns.findIndex(item => item.local?.id === item.id && item.text === turn.text) : -1;
    const local = turns[index]?.local;
    const received = local ? { ...turn, local: { ...local, sent: true } } : turn;
    return index < 0 ? [...turns, received] : turns.map((item, i) => i === index ? received : item);
  }, current);
}
export function Chat({ base, id, title, subtitle, working, onStatus }: { base: string; id: string; title: string; subtitle: string; working?: boolean; onStatus?: (session: Session) => void }) {
  const [history, setHistory] = useState<{ turns: ChatTurn[]; truncated: boolean }>({ turns: [], truncated: false });
  const [historyNoticeDismissed, setHistoryNoticeDismissed] = useState(false);
  const [connection, setConnection] = useState<Connection>('catching up');
  const [streamError, setStreamError] = useState('');
  const [sendError, setSendError] = useState('');
  const [waitingForReply, setWaitingForReply] = useState(false);
  const [draft, setDraft] = useState('');
  const [attempt, setAttempt] = useState(0);
  const bottom = useRef<HTMLDivElement>(null);
  useEffect(() => {
    setConnection('catching up'); setStreamError('');
    const stream = new EventSource(`${base}/events`);
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
    receive('snapshot', data => {
      const snapshot = snapshotSchema.parse(data);
      const ids = new Set(snapshot.turns.map(turn => turn.id));
      setHistory(old => ({ ...snapshot, turns: mergeTurns(old.turns.filter(turn => turn.local?.id === turn.id || ids.has(turn.id)), snapshot.turns) }));
      setWaitingForReply(snapshot.turns.at(-1)?.role === 'user');
      setConnection('live'); setSendError('');
    });
    receive('turn', data => {
      const turn = turnSchema.parse(data);
      setWaitingForReply(turn.role === 'user');
      setHistory(old => {
        const turns = mergeTurns(old.turns, [turn]);
        return { turns: turns.filter((item, index) => index >= turns.length - RECENT_TURN_LIMIT || item.local?.id === item.id), truncated: old.truncated || turns.length > RECENT_TURN_LIMIT };
      });
    });
    receive('status', data => {
      const status = sessionSchema.parse(data);
      if (status.id !== id) throw new Error('Status session does not match stream');
      onStatus?.(status);
    });
    return () => stream.close();
  }, [base, id, onStatus, attempt]);
  useEffect(() => {
    if (!history.truncated) return;
    const timer = setTimeout(() => setHistoryNoticeDismissed(true), HISTORY_NOTICE_MS);
    return () => clearTimeout(timer);
  }, [history.truncated]);
  const latest = history.turns.at(-1);
  useLayoutEffect(() => { bottom.current?.scrollIntoView({ block: 'end' }); }, [history.turns.length, latest?.local?.id ?? latest?.id, latest?.text]);
  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!draft.trim() || connection !== 'live') return;
    const localId = crypto.randomUUID();
    const message: ChatTurn = { id: localId, role: 'user', text: draft, local: { id: localId, sent: false } };
    setDraft(''); setSendError(''); setWaitingForReply(true);
    setHistory(old => ({ ...old, turns: [...old.turns, message] }));
    try {
      await sendMessage(base, { text: message.text });
      setHistory(old => ({ ...old, turns: old.turns.map(turn => turn.local?.id === localId ? { ...turn, local: { id: localId, sent: true } } : turn) }));
      setSendError('');
    } catch (cause) {
      setHistory(old => ({ ...old, turns: old.turns.filter(turn => turn.id !== localId) }));
      setWaitingForReply(false);
      // Preserve anything typed while this request was in flight.
      setDraft(current => current ? `${current}\n${message.text}` : message.text);
      setSendError(`Send was not confirmed. Draft restored; check the transcript before sending again. ${String(cause)}`);
    }
  }
  const replying = (working ?? waitingForReply) && connection === 'live';
  return <section class="chat" aria-label={`Chat with ${title}`}>
    <header><div><h2>{title}</h2><small>{subtitle}</small></div>
      <span class={`pill ${connection === 'live' ? 'live' : ''}`} role="status">{connection}</span></header>
    {connection === 'reconnecting' && <p class="banner" role="alert">Cannot reach Niles. Check that you’re connected to your tailnet.</p>}
    {streamError && <p class="banner" role="alert">{streamError} <button onClick={() => setAttempt(old => old + 1)}>Reconnect</button></p>}
    {sendError && <p class="banner" role="alert">{sendError}</p>}
    {history.truncated && !historyNoticeDismissed && <p class="banner">Showing the most recent {RECENT_TURN_LIMIT} turns. Earlier history is available at the desk.</p>}
    <div class="transcript" aria-label="Transcript" aria-live="polite">
      {history.turns.map(turn => <article class={`bubble ${turn.role}`} key={turn.local?.id ?? turn.id}>
        <small>{turn.role === 'agent' ? 'Agent' : turn.local ? turn.local.sent ? 'You · Sent' : 'You · Sending…' : 'You'}</small><Markdown text={turn.text} />
      </article>)}
      {(waitingForReply || working) && <div class="bubble reply-wait" role="status" aria-label="Waiting for reply"
        aria-hidden={!replying} style={{ visibility: replying ? 'visible' : 'hidden' }}>
        <span aria-hidden="true" /><span aria-hidden="true" /><span aria-hidden="true" />
      </div>}
      <div ref={bottom} />
    </div>
    <form onSubmit={submit}>
      <label for="message">Message agent</label>
      <div class="composer"><textarea id="message" rows={2} value={draft} onInput={event => setDraft(event.currentTarget.value)} />
        <button disabled={connection !== 'live' || !draft.trim()} type="submit">Send</button></div>
    </form>
  </section>;
}
