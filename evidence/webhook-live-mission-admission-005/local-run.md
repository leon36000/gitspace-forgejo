# WEBHOOK-LIVE-MISSION-ADMISSION-005 — événement Forgejo vers admission versionnée

Date : 24 octobre 2026. Exécution locale seulement, sur le worktree du checkpoint.

## Résultat

Une instance Forgejo 17 privée a émis l’événement `issues/opened` de son issue #1 vers le receiver Rust local. Le UUID de la tâche Forgejo (`cd261225-09e8-4c8c-a1af-be086956f9c5`) correspond à l’identifiant du journal du receiver; Forgejo a observé HTTP 202. `forgejo-mission-local` a lu l’événement, l’issue ouverte, la branche `main` marquée protégée, l’exigence et `src/lib.rs` au commit de base `1a7b649ac8e33e3d16a1aa1ad175457d39cae083`, puis produit une admission hôte versionnée `webhook_delivery/claimed` pour `MISSION-012-CLAMP-001`.

Le compte de lecture est non administrateur, sans droit push, et son jeton est limité aux portées `read:repository,read:issue`. Un mauvais jeton produit `mission_source_unauthorized` avant la création du journal de mission ou du fichier d’admission. Le rejeu du même delivery retourne `already_claimed`, garde `allocation_replayed=true` et ne change ni le hash du journal de mission ni celui de l’admission. Le seul delivery reste présent dans l’inbox. Le hook a été désactivé avec readback Forgejo; les processus suivis sont sortis et aucun listener ne subsiste sur les ports vérifiés.

Le problème logiciel est une implémentation réellement incorrecte de `clamp_to_range` : la valeur supérieure à `maximum` est renvoyée comme `minimum`; l’exigence et le test d’acceptation protégé sont déposés dans le dépôt Forgejo privé. **Ce run s’arrête à l’admission : il n’a lancé aucun agent, n’a pas modifié le code, créé de PR, exécuté la CI d’acceptation, vérifié un candidat ni fusionné.** Un autre parcours local antérieur couvre une correction et un flux PR/gate, mais il n’est pas composé avec cet événement dans cette exécution.

## Reproduction et provenance

Le binaire local indiqué au script était `/tmp/forgejo-17.0.0`, version affichée `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le dépôt officiel Forgejo `v17.0.0` a été retrouvé au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; ce tag n’établit pas à lui seul la provenance de construction du binaire local.

HEAD examiné : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; sources modifiées/non suivies présentes et leur SHA-256 sont consignés dans `provenance.json` (47 sources). Le receiver et le CLI d’admission sont compilés par le helper Bubblewrap hors réseau et hors HOME hôte, à partir d’un snapshot Cargo local avec verrouillage/offline. Leurs empreintes de binaires et le hash de sortie du build sont enregistrés. Les secrets ne sont consignés ni dans la sortie ni dans les preuves.

Commande d’exécution :

```text
python3 scripts/run_forgejo_live_webhook_mission_admission.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace --state-dir /tmp/forgejo-live-mission-webhook-008-20261024-05 --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/webhook-live-mission-admission-005
```

Vérifications relancées après l’exécution : 7/7 tests du nouveau driver, 6/6 tests du flux webhook émis, `py_compile`, zéro espace final dans les deux nouveaux fichiers Python, contrôle de l’état licence Cargo, et `sha256sum -c SHA256SUMS` dans le répertoire de preuves. Les sorties unitaires et contrôles sont conservés ici. Le contrôle Cargo confirme `license=null`, `license_file=null`, `publish=[]`, décision `unselected`, droits de redistribution `not_assessed`.

## Essais de mise au point conservés

Les quatre premiers essais restent dans `webhook-live-mission-admission-001` à `-004`; ils ne sont pas comptés comme réussites. 001/002 ont échoué avant création du hook, car le driver appelait une fonction de readiness sur le mauvais module. 003 a attendu le port sondé avant démarrage alors que le helper choisissait son propre port. 004 a bien provoqué un événement Forgejo, mais le receiver a répondu HTTP 401 : le secret binaire écrit dans son fichier différait du secret hexadécimal texte fourni à Forgejo. Le readback interne SQLite et l’absence de journal sont dans [le diagnostic 004](../webhook-live-mission-admission-004/diagnosis.json). 005 utilise l’adresse et le secret effectivement partagés.

## Limites et suite

Le profil local ne vérifie que le marqueur `protected` de la branche; l’adaptateur n’effectue pas de readback indépendant de la règle serveur `requirements/**`. Le compte hôte, Forgejo, le receiver et le CLI de confiance partagent l’UID du système. Le code candidat est téléchargé comme contexte, mais il n’est pas exécuté dans ce run. Le readback de tâche `hook_task` est une table SQLite interne Forgejo v17, pas une API supportée. L’allowlist webhook `127.0.0.1` est limitée à l’instance jetable.

Aucun AgentOS, LLM, RAG ou service mémoire n’est invoqué. Aucun modèle ni budget de tokens n’a été utilisé; aucun gain général n’est inféré. Les événements doublés/désordonnés, crash avant/après admission, base déplacée, changement sémantique concurrent, vérification indépendante et fusion dans la même chaîne restent à composer. Le hook, le serveur et le receiver ont été arrêtés; le répertoire d’état privé `/tmp/forgejo-live-mission-webhook-008-20261024-05` est conservé pour reproduction et n’a pas été supprimé.
