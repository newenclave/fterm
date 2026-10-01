# fterm shell integration for bash. Add this line to ~/.bashrc:
#   [ "$TERM_PROGRAM" = fterm ] && source /path/to/fterm.bash

[ "$TERM_PROGRAM" = fterm ] || return 0
[ -n "$__FTERM_LOADED" ] && return 0
__FTERM_LOADED=1
__fterm_running=

# The VS Code escape for OSC 633;E: `\` -> `\\`, `;` -> `\x3b`, a new line -> `\x0a`.
__fterm_escape() {
  local s=$1
  s=${s//\\/\\\\}
  s=${s//;/\\x3b}
  s=${s//$'\n'/\\x0a}
  printf '%s' "$s"
}

# The folder for OSC 7. Git Bash (MSYS) says `/c/work`; fterm needs `C:/work`, so ask cygpath,
# only when the folder changes (a new process is slow there).
__fterm_cwd_for=
__fterm_cwd_was=
__fterm_cwd() {
  if command -v cygpath >/dev/null 2>&1; then
    if [ "$PWD" != "$__fterm_cwd_for" ]; then
      __fterm_cwd_for=$PWD
      __fterm_cwd_was=$(cygpath -m "$PWD")
    fi
    printf '%s' "$__fterm_cwd_was"
  else
    printf '%s' "$PWD"
  fi
}

__fterm_preexec() {
  # The DEBUG trap runs for every command; only the first one after the prompt counts.
  [ -n "$COMP_LINE" ] && return
  [ "$BASH_COMMAND" = "$PROMPT_COMMAND" ] && return
  if [ -z "$__fterm_running" ]; then
    __fterm_running=1
    # The whole line, for the history (BASH_COMMAND is only one part of it).
    local line
    line=$(HISTTIMEFORMAT= builtin history 1)
    line=${line#*[0-9]  }
    [ -n "$line" ] && printf '\033]633;E;%s\007' "$(__fterm_escape "$line")"
    printf '\033]133;C\007'
  fi
}

__fterm_precmd() {
  local code=$?
  if [ -n "$__fterm_running" ]; then
    printf '\033]133;D;%s\007' "$code"
    __fterm_running=
  fi
  # `file://host/C:/work` (Git Bash) or `file://host/home/me`: one `/` after the host.
  local cwd
  cwd=$(__fterm_cwd)
  printf '\033]7;file://%s/%s\007' "$HOSTNAME" "${cwd#/}"
  printf '\033]133;A\007'
}

PROMPT_COMMAND="__fterm_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
PS1="$PS1"'\[\033]133;B\007\]'
trap '__fterm_preexec' DEBUG
