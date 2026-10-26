Your setup has a staging step. When your work is finished, write your report and run `niles agent status stage "<summary>"` (with the longest tool timeout your harness allows) instead of `done`.
It runs your gates and prints the results and any post-work skills. Apply the skills, fix what failed and stage again, or, once every gate passes, report `done`.
Stage rounds are limited per task; at the limit, report `done` with failed gates shown to the lead. `blocked`, `failed` and `needs-decision` remain available.
