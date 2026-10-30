// What ReUI's timer field (Win+Ctrl+T) makes of a line, for the page's demo: a subset of
// the app's grammar (crates/reui-hub/src/modules/timers/parse.rs and remind/grammar.rs) and
// the same preview its card shows (timers/view.rs `launcher`, remind/card.rs `preview`).

export type Parsed =
  | { kind: 'timer'; ms: number; label: string | null }
  | { kind: 'focus' }
  | { kind: 'stopwatch' }
  | { kind: 'reminder'; at: Date; text: string | null; day: string | null; every: string | null }
  | { kind: 'empty' }
  | { kind: 'unknown' };

export interface Preview {
  eyebrow: string;
  hero: string;
  detail: string;
}

const MINUTE = 60_000;
const capital = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
const words = (s: string) => s.trim().replace(/\s+/g, ' ');

/** A length on its own: `10`, `10m`, `90s`, `1h30`, `2h`, `5:00`. */
function length(token: string): number | null {
  let m: RegExpMatchArray | null;
  if ((m = token.match(/^(\d+)$/))) return Number(m[1]) * MINUTE;
  if ((m = token.match(/^(\d+)(m|min|mins)$/))) return Number(m[1]) * MINUTE;
  if ((m = token.match(/^(\d+)(s|sec|secs)$/))) return Number(m[1]) * 1000;
  if ((m = token.match(/^(\d+)h(\d+)?$/))) return Number(m[1]) * 60 * MINUTE + Number(m[2] ?? 0) * MINUTE;
  if ((m = token.match(/^(\d+):(\d\d)$/))) return Number(m[1]) * MINUTE + Number(m[2]) * 1000;
  return null;
}

/** The next time `hour:minute` comes round; a bare 12-hour hour takes the next of am/pm. */
function next(now: Date, hour: number, minute: number, meridiem: string | null, bare: boolean): Date {
  const at = new Date(now);
  at.setSeconds(0, 0);
  if (meridiem) {
    const h = (hour % 12) + (meridiem === 'pm' ? 12 : 0);
    at.setHours(h, minute);
    if (at <= now) at.setDate(at.getDate() + 1);
    return at;
  }
  at.setHours(hour, minute);
  if (bare && hour <= 12) {
    // "at 6": the next 6 o'clock to come round, morning or evening.
    const evening = new Date(at);
    evening.setHours(hour % 12 + 12);
    if (at <= now) return evening > now ? evening : (at.setDate(at.getDate() + 1), at);
    return at;
  }
  if (at <= now) at.setDate(at.getDate() + 1);
  return at;
}

export function parse(input: string, now = new Date()): Parsed {
  const line = words(input.toLowerCase());
  if (!line) return { kind: 'empty' };
  if (line === 'focus') return { kind: 'focus' };
  if (line === 'sw' || line === 'stopwatch') return { kind: 'stopwatch' };

  // A timer: one length token, the rest is its label. Alone, 5:00 is five minutes.
  const tokens = line.split(' ');
  const lengths = tokens.map(length);
  const which = lengths.findIndex((ms) => ms !== null);
  const clockLike = /^\d{1,2}:\d\d$/.test(tokens[which] ?? '');
  if (which >= 0 && (!clockLike || tokens.length === 1) && !/\b(at|in|every|tomorrow|tonight|weekdays)\b/.test(line)) {
    const label = tokens.filter((_, i) => i !== which).join(' ');
    return { kind: 'timer', ms: lengths[which]!, label: label ? label : null };
  }

  // A reminder: a time (at 6, 9:30, 6pm, noon) or a delay (in 2h), maybe a day or a repeat.
  let rest = ` ${line} `;
  let every: string | null = null;
  let day: string | null = null;
  if (/ every day | daily /.test(rest)) { every = 'Every day'; rest = rest.replace(/ every day | daily /, ' '); }
  if (/ weekdays /.test(rest)) { every = 'Weekdays'; rest = rest.replace(/ weekdays /, ' '); }
  if (/ tomorrow /.test(rest)) { day = 'Tomorrow'; rest = rest.replace(/ tomorrow /, ' '); }
  const tonight = / tonight /.test(rest);
  if (tonight) rest = rest.replace(/ tonight /, ' ');

  let at: Date | null = null;
  let m: RegExpMatchArray | null;
  if ((m = rest.match(/ in (\d+)\s*(h|hours?|m|min|mins|minutes?|days?) /))) {
    const n = Number(m[1]);
    const unit = m[2][0];
    at = new Date(now.getTime() + n * (unit === 'h' ? 60 * MINUTE : unit === 'd' ? 1440 * MINUTE : MINUTE));
    if (unit === 'd') day = n === 1 ? 'Tomorrow' : at.toLocaleDateString([], { weekday: 'short', day: 'numeric', month: 'short' });
    rest = rest.replace(m[0], ' ');
  } else if ((m = rest.match(/ noon /))) {
    at = next(now, 12, 0, null, false);
    rest = rest.replace(m[0], ' ');
  } else if ((m = rest.match(/ (?:at )?(\d{1,2})(?::(\d\d))?\s*(am|pm) /))) {
    at = next(now, Number(m[1]), Number(m[2] ?? 0), m[3], false);
    rest = rest.replace(m[0], ' ');
  } else if ((m = rest.match(/ (?:at )?(\d{1,2}):(\d\d) /))) {
    at = next(now, Number(m[1]), Number(m[2]), null, false);
    rest = rest.replace(m[0], ' ');
  } else if ((m = rest.match(/ at (\d{1,2}) /)) || ((day || tonight || every) && (m = rest.match(/ (\d{1,2}) /)))) {
    const hour = Number(m[1]);
    // On another day a bare hour reads as 7–11 morning, 12–6 afternoon; tonight is evening.
    const h = tonight ? (hour % 12) + 12 : day ? (hour >= 7 && hour <= 11 ? hour : (hour % 12) + 12) : hour;
    at = day || tonight ? next(now, h, 0, null, false) : next(now, hour, 0, null, true);
    rest = rest.replace(m[0], ' ');
  }
  if (!at) return { kind: 'unknown' };
  if (day === 'Tomorrow') {
    const t = new Date(now);
    t.setDate(t.getDate() + 1);
    at.setFullYear(t.getFullYear(), t.getMonth(), t.getDate());
  }
  // A time that has passed today comes round tomorrow, and says so.
  if (!day && !every && at.toDateString() !== now.toDateString()) {
    const t = new Date(now);
    t.setDate(t.getDate() + 1);
    day = at.toDateString() === t.toDateString() ? 'Tomorrow' : at.toLocaleDateString([], { weekday: 'short' });
  }
  const text = words(rest);
  return { kind: 'reminder', at, text: text ? capital(text) : null, day, every };
}

/** `m:ss` below an hour, `h:mm:ss` from there (timers/view.rs `clock_text`). */
export function clock(ms: number): string {
  const secs = Math.ceil(ms / 1000);
  const h = Math.floor(secs / 3600);
  const m = Math.floor(secs / 60) % 60;
  const s = secs % 60;
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}` : `${m}:${String(s).padStart(2, '0')}`;
}

export const timeOfDay = (d: Date) => d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

/** What the field's card says before Enter. */
export function preview(p: Parsed): Preview {
  switch (p.kind) {
    case 'timer':
      return { eyebrow: 'Timer', hero: clock(p.ms), detail: p.label ?? '' };
    case 'focus':
      return { eyebrow: 'Focus', hero: 'Focus', detail: '' };
    case 'stopwatch':
      return { eyebrow: 'Stopwatch', hero: '0:00', detail: '' };
    case 'reminder': {
      const when = p.every ?? p.day;
      const detail = p.text && when ? `${p.text} · ${when}` : p.text ?? when ?? '';
      return { eyebrow: 'Reminder', hero: timeOfDay(p.at), detail };
    }
    case 'unknown':
      return { eyebrow: 'Timer', hero: '0:00', detail: 'Type a length, like 10m' };
    default:
      return { eyebrow: 'Timer', hero: '25:00', detail: '' };
  }
}
