/**
 * ANSI CSI escape sequences: colours, cursor moves, erase commands.
 * Parameter bytes are 0x30-0x3F (digits, `:;<=>?`), so colon-delimited
 * truecolor such as `ESC[38:2:255:100:0m` is covered too.
 */
const ANSI_CSI = /\u001b\[[0-?]*[ -/]*[@-~]/g;

/** Remove ANSI CSI escape sequences from terminal text. */
export function stripAnsi(str: string): string {
  return str.replace(ANSI_CSI, "");
}
