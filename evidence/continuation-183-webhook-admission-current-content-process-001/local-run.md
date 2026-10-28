# CONTINUATION-183 — admission de processus du contenu courant non fiable

Date du run : 2026-10-27. Le checkout produit reste sale et le `HEAD` observé avant le test est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le lot n’a créé ni commit ni branche.

## Objectif et résultat

Prouver au niveau processus, plutôt qu’uniquement par contrat Rust, que le receiver accepte un webhook signé contenant un titre/corps ancien et que le CLI d’admission obtient le contenu d’issue courant par sa lecture API. Le texte courant doit être transmis comme donnée d’utilisateur non fiable et rester distinct des exigences protégées.

Le scénario `tests/forgejo_mission_delivery_process_e2e.rs::actual_mission_cli_admits_signed_webhook_after_closed_refusal_and_reuses_allocation` passe : `1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out`. Le log brut de l’invocation est conservé dans `isolated-process-test.log`; les versions et empreintes des sources sont dans `source-hashes.txt`.

## Parcours observé

1. Le vrai binaire `forgejo-webhook-local` reçoit un corps synthétique signé HMAC `issues/opened`; le titre et le corps du webhook indiquent explicitement qu’ils sont périmés. Le receiver accepte la delivery, séquence 1.
2. Le vrai binaire `forgejo-mission-local` lit la delivery et refuse d’abord l’admission, car le fixture Forgejo répond que l’issue est fermée. Il ne crée alors ni allocation de mission ni handoff hôte.
3. Le fixture loopback est rouvert; son GET issue renvoie le titre `Current title from Forgejo API` et le corps `Current body from Forgejo API; this is optional untrusted context.`
4. Le même CLI admet la delivery. Le JSON de réponse contient ces valeurs dans `execution_request.untrusted_work_item`, avec `source=current_forgejo_issue_api`, `trust=untrusted_user_content` et la route API canonique. Les assertions vérifient que `requirements_json` et `requirement_sha256` sont exactement ceux des exigences protégées, et que le titre/corps périmés du webhook ne sont pas retenus. Le handoff versionné est aussi relu depuis le fichier écrit.
5. Le scénario complet vérifie le rejeu/la réutilisation de l’allocation et le contenu de l’inbox; le log rapporte deux deliveries et 31 requêtes HTTP au fixture pour toutes les assertions du scénario.

## Commande et isolation

Commande lancée :

```sh
cargo fmt --all && cargo fmt --all -- --check && git diff --check && python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_admits_signed_webhook_after_closed_refusal_and_reuses_allocation
```

Le runner construit hors ligne, sous Bubblewrap, les binaires Rust utilisés et le test ciblé. Il monte les sources en lecture seule, masque le home et `.runtime`, sépare les namespaces réseau et PID (`--unshare-net --unshare-pid`) et utilise Cargo offline. Le serveur API est un fixture HTTP loopback synthétique dans ce namespace; aucune instance Forgejo n’est jointe. L’environnement rapporté est Linux aarch64, `rustc 1.95.0`, `cargo 1.95.0`, Bubblewrap `0.9.0`; les empreintes complètes du runner et des sources se trouvent dans `isolated-process-test.log` et `source-hashes.txt`.

Le fait que la commande chaînée ait atteint et terminé le runner établit également le succès de `cargo fmt --all`, `cargo fmt --all -- --check` et `git diff --check` exécutés avant lui. Le résumé de sortie lisible est fourni séparément dans `test-result-summary.txt`.

## Limites

- Le serveur Forgejo est un fixture local, non une instance Forgejo réelle; le test est ciblé et séquentiel (`n=1`).
- Aucun appel `AgentRuntimeV1::execute`, AgentOS, Forge Code, RAG, moteur de mémoire ou LLM n’est observé. Le CLI émet un DTO d’exécution; aucun agent ne consomme ici le champ.
- Ce lot ne fait ni modification de code applicative, PR, vérification indépendante, décision de merge, ni fusion contrôlée. Il ne démontre pas l’atomicité entre la lecture de l’issue et son usage ultérieur.
- `source` et `trust` classent le champ dans le contrat local; ils ne constituent ni une attestation de la source ni une preuve de correction. Le contenu d’issue reste hostile et ne remplace pas les exigences protégées.
- Aucun benchmark comparatif, gain, mesure de tokens ou coût de fournisseur n’est établi.

Le projet reste incomplet. Cette preuve complète un niveau processus pour l’admission, mais ne ferme pas les critères de parcours avec un runtime agent réel ni de comparaison appariée.
