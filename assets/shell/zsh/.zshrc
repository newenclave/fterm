# fterm: an interactive zsh reads this file. ZDOTDIR goes back to your folder (so zsh reads your
# .zlogin and your history file there), then your .zshrc runs, then the fterm shell integration.
ZDOTDIR=${FTERM_USER_ZDOTDIR:-$HOME}
unset FTERM_USER_ZDOTDIR
[[ -f "$ZDOTDIR/.zshrc" ]] && builtin source "$ZDOTDIR/.zshrc"

export TERM_PROGRAM=fterm
builtin source "${__fterm_zdotdir:h}/fterm.zsh"
unset __fterm_zdotdir
