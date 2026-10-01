# fterm loads this file: `bash --rcfile fterm-rc.bash -i` (a normal interactive bash, for example on Linux).
# With --rcfile bash reads only this file, so it reads the system file and your ~/.bashrc here,
# and then loads the fterm shell integration.

[ -f /etc/bash.bashrc ] && . /etc/bash.bashrc
[ -f ~/.bashrc ] && . ~/.bashrc

export TERM_PROGRAM=fterm
. "$(dirname "${BASH_SOURCE[0]}")/fterm.bash"
