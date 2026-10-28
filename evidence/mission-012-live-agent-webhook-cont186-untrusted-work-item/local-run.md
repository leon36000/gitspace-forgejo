# CONTINUATION-186 — parcours composé avec le contenu d’issue courant

Date : 27 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, déjà fortement modifié. Aucune modification de code produit, aucun commit, publication ou accès externe dans ce lot.

## Exécution

Commande du superviseur :

```sh
python3 scripts/run_mission_012_supervised_private_netns_e2e.py \
  --state-dir /tmp/mission-012-cont186-untrusted-work-item \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-cont186-untrusted-work-item" \
  --execution-id live-mcp-186 \
  --allocation-id mission012-cont186-untrusted-work-item
```

Le superviseur a imposé Bubblewrap 0.9.0 et un namespace réseau différent de celui de l’hôte, avec l’instance Forgejo et son superviseur dans ce namespace privé. Forgejo v17.0.0 est épinglé par SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit exact de build est inconnu. L’hôte ne joignait pas le port loopback 3132 pendant que Forgejo vivait (`ECONNREFUSED`), et le test d’entrée dans son namespace était refusé (`EPERM`).

Le wrapper a fixé `--agent-adapter local-test-adapter --sandbox-test-adapter`, sans invocation LLM. Reçu : `supervised-private-netns-observation.json` (`verdict=passed`, toutes les assertions vraies); l’instance, le hook et son état ont été nettoyés par le runner, et la vérification de port absent a réussi. La sortie complète contrôlée et les résumés sont conservés dans ce dossier et son dossier `-admission`.

## Résultat

L’admission webhook a relu l’issue #1 du dépôt privé `forge-admin/mission-012-clamp` via `current_forgejo_issue_api`. L’enveloppe v1 est marquée `untrusted_user_content`, liée à `updated_at`, au dépôt, à la route et à un hash de contenu. Elle apparaît dans `admission.json` séparément du contexte de code et des exigences protégées. Le résumé d’exécution contient 8 reçus, dont `mission.context`, `cargo.test`, `workspace.read_text` et `workspace.write_text`; l’adaptateur a réussi et n’a modifié que `src/lib.rs`.

La vérification sur la base a d’abord échoué de façon actionnable : l’acceptation retournait `-2` au lieu de `10` (un autre test passait). L’adaptateur a corrigé la fonction de clamp; les deux tests d’acceptation ont ensuite passé sur la candidate et à la revérification pré-fusion. Le vérificateur indépendant `mission-012-rust-isolated-cargo-acceptance-v2` a rendu `pass` sur les SHA de base et candidate exacts. PR locale #2; tentative de merge avant revue refusée en HTTP 405 et tentative par reviewer non autorisé refusée en HTTP 405; le merge autorisé a créé `7599dea0f1108142cdf829bd58beb098553487d2`, tête `main` lue après merge. Le comportement sur ce commit exact a été revalidé (`2 passed; 0 failed`). Les événements PR ouverts, revue approuvée et fermée ont tous été relus vers l’état courant.

Le processus agent ne voyait ni credential de fusion, ni fichiers runtime privés; il ne joignait pas les routes API, Web et `git-receive-pack`, n’avait aucun réseau connecté et ne gardait que les FD 0–2. Le gate hôte a néanmoins autorisé le merge. Ceci est une observation locale du bac à sable et du gate utilisé; elle ne prouve pas l’exclusion de tous les writers concurrents ni la politique du superviseur, qui reste explicitement non vérifiée indépendamment.

## Portée et limites

Cette exécution compose le readback API courant, le webhook, l’admission, le host MCP, l’adaptateur, la PR, la vérification, l’autorisation de merge et le test post-merge sur une nouvelle instance Forgejo isolée (`n=1`). Coût modèle rapporté : `0.00 USD`; tokens : non applicables. Ce coût vaut pour l’absence d’appel modèle, pas pour la facturation du runner ou une comparaison de productivité.

Le rapport de composition ne conserve pas un booléen distinct `untrusted_work_item_observed` provenant de la sortie de l’adaptateur. Il conserve l’enveloppe admise et le reçu `mission.context`; la validation directe de l’adaptateur sur ce contrat et un contenu hostile reste celle de CONTINUATION-185. Ce run prouve la composition de l’interface dans le parcours réussi, sans prétendre à un test LLM. Aucun AgentOS, RAG ou moteur de mémoire réel n’a été utilisé. L’adaptateur déterministe, le même hôte et les privilèges root/compromission noyau hors modèle empêchent d’en déduire une garantie de production ou un gain général.

Détails reproductibles : `composed-run.json`, `composed-stage-summaries.json`, `admission.json`, `supervised-private-netns-observation.json`, reçus de vérification et `composition-source-snapshot.zip`. Les sommes de contrôle locales et globales sont append-only; `manifest-prefix-before.json` fixe leurs préfixes antérieurs.
