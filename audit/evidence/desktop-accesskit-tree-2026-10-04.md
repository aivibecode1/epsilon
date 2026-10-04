# Desktop accessibility tree, 2026-10-04

Dumped from `FullOutput::platform_output.accesskit_update` — the tree egui hands to the platform
adapter, which is what a screen reader consumes. Read rather than asserted at.

Window state: a real faux session, a goal (armed, round 2/8), the composer holding a typed draft,
1440x900, with accessibility enabled.

  id=13826925289788388 role=Window                 label=                                                     value=
  id=497332518733405788 role=GenericContainer       label=                                                     value=
  id=757430289277341234 role=TextRun                label=                                                     value=  cannot continue the goal: this window 
  id=1446607719953206068 role=Button                 label=Send                                                 value=
  id=1478608454685057808 role=MultilineTextInput     label=Composer                                             value=
  id=1903421509157723082 role=GenericContainer       label=                                                     value=
  id=2424181957214368090 role=GenericContainer       label=                                                     value=
  id=3355781303506654862 role=Pane                   label=Editor, read-only · no file open                     value=
  id=3911401198727858209 role=GenericContainer       label=                                                     value=
  id=3972557672278588901 role=TextRun                label=                                                     value=  cannot continue the goal: this window 
  id=4822065536991275446 role=Pane                   label=Agent transcript · 2 entries                         value=
  id=5330011884710027535 role=Label                  label=Transcript row · Notice · cannot continue the goal: this window is not running on an agent runtime value=  cannot continue the goal: this window 
  id=5576281950287361034 role=GenericContainer       label=                                                     value=
  id=5785966599777516553 role=GenericContainer       label=                                                     value=
  id=6129452616890722540 role=Pane                   label=Status bar · idle                                    value=
  id=6863435719390127133 role=GenericContainer       label=                                                     value=
  id=6968602126632576103 role=Pane                   label=Activity bar · showing files                         value=
  id=7296601255057362514 role=Pane                   label=Title bar · .tmpPYFFaY  ·  Faux (offline scripted)  ·  offline · ctx 1%*/1.0M  ·  ↑0 ↓0 value=
  id=7833124326465346503 role=GenericContainer       label=                                                     value=
  id=8479516623688059382 role=TextRun                label=                                                     value=
  id=8591658052727341957 role=Button                 label=sessions                                             value=
  id=8942438533293197971 role=TextRun                label=                                                     value=
  id=8993309008742828480 role=GenericContainer       label=                                                     value=
  id=9414540917939641855 role=Button                 label=subagents                                            value=
  id=10187431247178612995 role=GenericContainer       label=                                                     value=
  id=11172427599028083040 role=GenericContainer       label=                                                     value=
  id=11368364160594750606 role=Pane                   label=Workspace files · 1 files                            value=
  id=12660099232636891636 role=TextInput              label=                                                     value=
  id=12846694785769360241 role=GenericContainer       label=                                                     value=
  id=13883424466863211777 role=Pane                   label=Goal band · active · round 2/8 · 1 gate(s) · armed   value=
  id=14222615271836336626 role=GenericContainer       label=                                                     value=
  id=14514878516489269442 role=Label                  label=Transcript row · Notice · cannot continue the goal: this window is not running on an agent runtime value=  cannot continue the goal: this window 
  id=14723068226476680747 role=GenericContainer       label=                                                     value=
  id=15051880049312063873 role=Unknown                label=                                                     value=
  id=15714956561147064303 role=GenericContainer       label=                                                     value=
  id=16159285440268384037 role=GenericContainer       label=                                                     value=
  id=16508067512481630242 role=GenericContainer       label=                                                     value=
  id=16525113507552270008 role=Button                 label=files                                                value=
  id=18356943641707106210 role=GenericContainer       label=                                                     value=

Findings from reading it:

1. **\Workspace files · 1 files\** — plural bug, in the label a screen reader is told.
2. **A \TextInput\ with an empty label** — an input nobody can name.
3. **One transcript row announces its text three times** — a \Label\ node (named, good) plus two
   \TextRun\ selection runs carrying the same string as their value.

