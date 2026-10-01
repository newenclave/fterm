# fterm shell integration for zsh. Add this line to ~/.zshrc:
#   [[ "$TERM_PROGRAM" == fterm ]] && source /path/to/fterm.zsh

[[ "$TERM_PROGRAM" == fterm ]] || return 0
[[ -n "$__FTERM_LOADED" ]] && return 0
typeset -g __FTERM_LOADED=1
typeset -g __fterm_running=

__fterm_preexec() {
  __fterm_running=1
  printf '\033]133;C\007'
}

__fterm_precmd() {
  local code=$?
  if [[ -n "$__fterm_running" ]]; then
    printf '\033]133;D;%s\007' "$code"
    __fterm_running=
  fi
  printf '\033]7;file://%s%s\007' "$HOST" "$PWD"
  printf '\033]133;A\007'
}

autoload -Uz add-zsh-hook
add-zsh-hook preexec __fterm_preexec
add-zsh-hook precmd __fterm_precmd
PS1="$PS1"$'%{\033]133;B\007%}'
