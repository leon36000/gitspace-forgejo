# FORGEJO-NATIVE-COMMENT-ADMISSION-002

## Résultat

Le 26 octobre 2026, l’instance Forgejo v17 jetable a créé une issue privée puis un commentaire ordinaire `issue_comment` contenant une instruction hostile. Le hook Forgejo réellement configuré sur le sélecteur `issues` a émis l’événement `created`; le receiver local l’a journalisé à la séquence 2 et Forgejo a enregistré l’UUID correspondant avec HTTP 202. Le binaire `forgejo-mission-local`, compilé depuis le worktree courant, a relu cette livraison et a rendu le refus structuré `webhook_delivery_resource_mismatch` (exit 1).

Avant/après le refus, le snapshot des fichiers du projet (hors journal d’inbox qui reçoit naturellement l’événement), le fichier d’admission existant, le journal de mission existant et le contenu canonique de l’exigence versionnée sont identiques. La preuve établit donc qu’aucune nouvelle allocation ou revendication n’a été créée par le commentaire et que l’admission précédente n’a pas été altérée. L’exigence a été relue par l’API de contenu Forgejo avant et après le refus et son SHA-256 est identique. Les valeurs archivées sont des empreintes; aucun credential ni corps brut du commentaire n’est inclus dans le reçu spécialisé.

Le chemin d’exécution compile et le contrôle est soutenu par deux niveaux de preuve : le run observe le code d’erreur produit après lecture de l’inbox réelle; `src/bin/forgejo-mission-local.rs::admit_delivery` lit ensuite la livraison, vérifie la ressource, et retourne avant `read_local_context` si la ressource n’est pas une issue. `src/mission_admission.rs::admit_reconciled_issue_v2` garde le même contrôle de type avant relecture d’état ou admission. Cette observation d’exécution n’est pas une trace réseau prouvant zéro requête API; l’ordre du code montre que ce chemin de refus n’appelle pas `read_local_context`.

## Commande et environnement

```text
python3 scripts/run_forgejo_live_webhook_mission_admission.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-native-comment-admission-002 --evidence-dir evidence/forgejo-native-comment-admission-002 --native-issue-comment-refusal
```

Forgejo annonce `17.0.0+gitea-1.22.0`, construit avec Go 1.26.9. SHA-256 du binaire : `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le commit exact de construction de ce binaire n’est pas établi. Python 3.12.3, Rust/Cargo 1.95.0 et Bubblewrap 0.9.0. HEAD de base du worktree : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; tous les fichiers concernés sont des modifications locales non commités.

Résultat précis : séquence 1, `issues/opened`, HTTP 202; séquence 2, `issue_comment/created`, issue 1/commentaire 1, HTTP 202. Le refus a l’exit 1 et le code `webhook_delivery_resource_mismatch`. Le snapshot contient les empreintes avant/après des fichiers runtime protégés et du projet. Le hook a été désactivé puis relu; après arrêt, Forgejo et le receiver ont tous deux quitté proprement et aucun listener attendu ne subsistait.

## Vérifications

- `python3 -m unittest tests.test_forgejo_live_webhook_mission_admission` : 29 réussis.
- `python3 -m py_compile scripts/run_forgejo_live_webhook_mission_admission.py tests/test_forgejo_live_webhook_mission_admission.py` : réussite.
- `git diff --check` : réussite.
- Le run a recompilé les binaires de mission et de receiver dans le bac isolé avec Cargo verrouillé/hors-ligne; les hashes source et binaires sont dans `provenance.json`.
- Le manifeste local couvre tous les fichiers de ce dossier; les index globaux vérifient leurs nouvelles empreintes tout en gardant leurs préfixes append-only.

Les principaux hashes source à la compilation sont dans `provenance.json`; notamment le runner `scripts/run_forgejo_live_webhook_mission_admission.py` a SHA-256 `403080dcdaaad42ae4f48c2775cda12f0f2cf967913f658981120090e784b793`, le CLI mission `src/bin/forgejo-mission-local.rs` `b3a31a33b02a215f121548e9004ac569b5bdd241512f60cb4dce4f4e83f5b672`, et `src/mission_admission.rs` `2cba1de794d67605de10c94fe1fa33266e66af0a7b4efc58e1253b98d00ebd8b`.

Un premier essai conservé dans [FORGEJO-NATIVE-COMMENT-ADMISSION-001](../forgejo-native-comment-admission-001/local-run.md) s’est arrêté dans le runner parce qu’il supposait à tort que l’exigence Forgejo était copiée dans le workspace temporaire. La relecture de l’exigence a été corrigée vers l’API de contenu Forgejo; le second run ci-dessus a passé. Aucun scratch ni dossier de preuve antérieur n’a été supprimé.

## Limites

Un seul run sur une instance locale, avec `local-test-adapter` absent et aucun agent LLM invoqué. L’événement hostile n’a jamais été envoyé à un modèle. Le test démontre ce refus pour une livraison de commentaire ordinaire sur cette version de Forgejo et ce chemin de code; il ne démontre pas la sûreté d’un consommateur qui lirait le texte, l’isolement contre tous les writers, une PR ou une fusion. Le hash du binaire Forgejo n’établit pas son commit de construction. Aucun coût modèle, benchmark comparatif, AgentOS, RAG ou mémoire n’a été exercé; aucun gain général n’est conclu.
