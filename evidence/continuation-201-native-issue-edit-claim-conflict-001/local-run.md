# CONTINUATION-201 — édition native Forgejo après revendication d’allocation

## Résultat observé

Le test processus a utilisé l’instance locale jetable Forgejo `17.0.0+gitea-1.22.0` et un hook `issues` réel. Après l’événement `issues/opened`, l’admission du contexte actuel et un retry identique, le scénario modifie le titre de l’issue via l’API locale. Forgejo émet une seconde delivery `issues/edited`; son `changes.title.from` correspond au titre antérieur, son titre courant correspond au GET API et son delivery UUID est distinct. Le hook reçoit HTTP 202 et lie l’événement à la séquence receiver 2.

Le vrai binaire `forgejo-mission-local` est relancé avec le même allocation ID et la nouvelle delivery. Il relit l’issue courante puis sort 1 avec `verdict=inconclusive` et `execution_idempotency_conflict`; le validateur du runner classe cette réponse comme refusée et lie sa preuve à l’événement. L’admission déjà revendiquée, le journal de mission, l’exigence versionnée et l’inbox ne changent pas pendant cet appel. Les reçus JSON limitent l’événement aux hashes du titre, à son lien `changes.from` et aux identifiants de livraison; ils ne contiennent pas les valeurs de credentials. Voir `forgejo-live/native-issue-edit-event.json`, `native-issue-edit-refusal.json`, `result.json` et le reçu du runner dans ce dossier.

Commande reproductible :

```sh
python3 scripts/run_forgejo_live_webhook_mission_admission.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --repo-root . \
  --state-dir /tmp/cont201-native-issue-edit-state \
  --evidence-dir evidence/continuation-201-native-issue-edit-claim-conflict-001/forgejo-live \
  --native-issue-edit-conflict
```

Le binaire Forgejo a le SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit exact de construction n’est pas établi. La provenance donne le HEAD de la source exécutée (`f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`), le hash du runner, du test, des sources compilées et du lockfile. Toolchain et hashes : `toolchain-and-source-hashes.json`.

## Tests

Le test-first est conservé dans `red.log`: avant l’ajout des validateurs et de l’option, quatre nouveaux cas échouaient précisément sur les fonctions/attributs absents. Après implémentation, le module ciblé passe 36/36 (`green-unit.log`); la découverte Python complète passe 367/367 (`python-full-suite.log`). `cargo test --offline --locked --all-targets` passe avec 661 réussis, 0 échec et 124 ignorés sur 36 binaires (`cargo-all-targets.log`). `py_compile`, `cargo fmt --all -- --check`, `git diff --check` et le scan d’espaces terminaux passent.

## Portée et limites

`n=1`, issue #1 et allocation unique sur un Forgejo local. L’instance et le receiver se sont arrêtés; le readback confirme les processus terminés et les ports absents. Aucun AgentRuntime, LLM, AgentOS, PR, vérification indépendante, merge ou comportement post-fusion n’a été exécuté. Le test n’établit aucun gain de productivité. Il ne modifie pas le comportement de production Rust; il ajoute au runner une option de refus natif testable et ses validateurs.

Le répertoire privé généré `/tmp/cont201-native-issue-edit-state` est conservé conformément à l’interdiction de suppression : racine et `.runtime` en mode 0700, fichiers privés observés en mode 0600. Les valeurs elles-mêmes ne sont pas copiées dans les preuves; aucune lecture de ces valeurs n’est nécessaire. Aucun service externe, coût modèle ni environnement de production n’a été utilisé; aucun code source Forgejo upstream n’a été copié. L’instance locale a utilisé le binaire Forgejo déjà présent.
