# CONTINUATION-213 — interruption du CLI entre lecture PR et lecture commentaire

## Résultat

Le worktree était au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; aucune réinitialisation, aucun commit ni nettoyage n’a été effectué. Une instance Forgejo v17.0.0 jetable a émis deux webhooks natifs, `pull_request/review_requested` puis `issue_comment/created` (`is_pull=true`), pour une PR privée de fixture. Le receiver Rust les a conservés dans le même journal. Forgejo a été lancé sur `127.0.0.1:3131`; un proxy local à destination fixe `127.0.0.1:3131` a exposé le profil Rust attendu sur `127.0.0.1:3130`.

Pendant la réconciliation, le proxy a autorisé exactement les GET de cette PR et du commentaire par ID. Une tentative POST sur la route commentaire, avec le même credential read-only, a reçu 403 au proxy et n’a pas été transmise à Forgejo. Le premier CLI a terminé le GET courant de la PR; le proxy a ensuite retenu le GET commentaire avant tout transfert amont. Le harnais a envoyé SIGKILL au groupe de processus isolé du CLI, a vérifié le retour `-9` et l’absence du groupe, puis a abandonné cette requête au proxy avec HTTP 503 local.

Le journal webhook est resté octet pour octet identique pendant l’interruption (SHA-256 `8aa7c224b7548b72c6d74864cad47c48c158a740217675baa033b9847744b751`). Une nouvelle invocation du CLI, avec les mêmes IDs et le même journal, a relu la PR et le commentaire dans l’ordre, retourné `current_state_read`, conservé l’équipe reviewer et le label `untrusted_user_content`, puis laissé le journal inchangé. La projection ordonnée des 16 accès API Forgejo correspond à celle des requêtes transmises par le proxy; aucun GET commentaire amont ne précède la reprise. La PR reste ouverte et non fusionnée.

Les reçus et le vérificateur sont sous [live-e2e-001](live-e2e-001/result.json), [rapport de reprise](live-e2e-001/interrupted-cli-resume.json), [requêtes du proxy](live-e2e-001/api-proxy-requests.json), [projection du journal Forgejo](forgejo-access-log-projection.json) et [contrôle indépendant](verify_run_result.py). Forgejo est `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; son commit de build exact reste inconnu. Le répertoire d’état privé conservé dans `/tmp` est `/tmp/forgejo-team-review-cont213-state-001` (mode `0700`). Les deux services et le proxy ont été arrêtés, et les listeners sur 3130, 3131 et le port du receiver sont absents.

## Vérifications

- Le module de tests du harnais passe `29/29`. Le test-first initial échouait sur l’import du nouveau type de barrière, avant son implémentation.
- Le garde de métadonnées Cargo passe `12/12`; `cargo metadata --locked --offline --no-deps` conserve `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`.
- Le runner reconstruit les deux exécutables Rust avec Cargo `1.95.0`, Rust `1.95.0` et Bubblewrap `0.9.0`, en mode offline/locked, sources et toolchain en lecture seule, sans réseau ni PID partagés pendant la compilation. Python `3.12.3` exécute le harnais.
- `py_compile`, le vérificateur indépendant du reçu et `git diff --check` passent. Les commandes sont dans [commands.txt](commands.txt); les empreintes sources sont dans [source-sha256.txt](source-sha256.txt).

## Portée et limites

Un run live (`n=1`) démontre l’annulation/reprise du CLI Rust contre Forgejo local pour ce batch et ces deux routes. Il ne démontre pas un checkpoint durable du réconciliateur, une panne de machine/disque, la durabilité multi-hôte, l’isolation réseau OS du processus, la résistance à tout contournement du proxy, une autorité de fusion, un agent LLM/AgentOS, ni un gain comparatif. La liste de routes est imposée par le proxy du harnais, pas par une frontière réseau de production. Les données sont synthétiques et locales; aucun service externe ni coût de modèle n’a été utilisé.

Cette tranche ne crée pas de licence produit et n’évalue pas les droits de redistribution. La licence GitSpace demeure indéterminée et aucun code GitSpace ou Forgejo n’est copié. Les exigences de PR logicielle candidate, vérification indépendante, décision motivée et fusion contrôlée restent à démontrer dans un parcours complet; les critères cumulatifs et le benchmark apparié restent ouverts.
