# fterm: a login zsh reads this file. It loads your .zprofile (see .zshenv).
[[ -f "${FTERM_USER_ZDOTDIR:-$HOME}/.zprofile" ]] && builtin source "${FTERM_USER_ZDOTDIR:-$HOME}/.zprofile"
