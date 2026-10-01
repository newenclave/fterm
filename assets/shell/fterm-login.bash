# fterm loads this file in WSL: `bash --rcfile fterm-wsl.bash -i`.
# It does what a login bash does (your profile files), and then loads the fterm shell integration.

[ -f /etc/profile ] && . /etc/profile
if [ -f ~/.bash_profile ]; then
  . ~/.bash_profile
elif [ -f ~/.bash_login ]; then
  . ~/.bash_login
elif [ -f ~/.profile ]; then
  . ~/.profile
elif [ -f ~/.bashrc ]; then
  # No profile file loads it, so load it here.
  . ~/.bashrc
fi

export TERM_PROGRAM=fterm
. "$(dirname "${BASH_SOURCE[0]}")/fterm.bash"
