-- fterm config for a portable fterm: fterm.exe finds this file next to it.
-- All the data of fterm (the history, the sessions, the shell scripts) goes into the "data" folder here.
-- More settings: https://github.com/newenclave/fterm/blob/main/docs/CONFIG.md
-- (or press Ctrl+Shift+, in fterm to open this file).
return {
  data_dir = "data",

  -- font = { size = 14 },
  -- gpu = { backend = "gl" },   -- the least memory; "auto" = DX12 on Windows
}
