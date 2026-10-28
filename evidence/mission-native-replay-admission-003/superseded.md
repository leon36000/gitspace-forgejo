# Run superseded

Ce run a vérifié les tâches Forgejo, le replay natif et les empreintes avant/après de l’état mission, mais ne liait pas le hash sémantique de chaque payload reçu au journal à celui de sa task Forgejo. La revue read-only a relevé cette lacune; le runner et un test de champ secondaire divergent ont été corrigés, puis le comportement a été réexercé dans [le run final 004](../mission-native-replay-admission-004/local-run.md). Ce dossier est conservé comme historique, pas comme preuve finale du binding payload reçu.
