# fterm starts zsh with ZDOTDIR on this folder, so zsh reads these files instead of yours.
# Each one loads your own file. `.zshrc` then loads the fterm shell integration and gives ZDOTDIR back.
# Your folder is in FTERM_USER_ZDOTDIR (empty = your home folder).

__fterm_zdotdir=$ZDOTDIR
ZDOTDIR=${FTERM_USER_ZDOTDIR:-$HOME}
[[ -f "$ZDOTDIR/.zshenv" ]] && builtin source "$ZDOTDIR/.zshenv"
# Your .zshenv can set ZDOTDIR: keep it for the next files.
FTERM_USER_ZDOTDIR=$ZDOTDIR
if [[ -o interactive ]]; then
  ZDOTDIR=$__fterm_zdotdir
else
  # `zsh -c ...` reads no more files: give ZDOTDIR back now.
  unset FTERM_USER_ZDOTDIR __fterm_zdotdir
fi
