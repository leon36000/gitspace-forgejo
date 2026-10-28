# WEBHOOK-BRANCH-404-BATCH-CACHE-001 — cache négatif au niveau processus

## Résultat

Un test processus ajouté traverse le receiver HMAC, le journal durable, le CLI forgejo-reconcile-local et la fixture HTTP loopback. Il soumet deux notifications push avec des IDs de livraison distincts qui désignent la même branche supprimée `feature/release/agent-proof`. L’API synthétique répond 404. La réconciliation ne fait qu’un GET, garde l’ordre des deux résultats, retourne pour chacun `inconclusive/resource_not_found` non réessayable avec l’action qui rappelle l’ambiguïté 404, ne publie aucun état canonique et ne réécrit pas l’inbox. Le corps d’erreur et tout en-tête Authorization sont absents des sorties/requêtes observées.

Le scénario ciblé passe 1/1 sous le runner Bubblewrap, qui impose `--unshare-net` et sert l’API sur loopback. La matrice isolée complète sélectionne les 44 scénarios déclarés par le runner et passe 44/44. La suite `cargo test --offline --locked --all-targets -- --test-threads=1` passe 506 tests, 0 échec, 59 ignorés sur 34 groupes. `cargo fmt --all -- --check`, `git diff --check`, la syntaxe Python du runner, le readback de licence et ses 7 tests passent.

## Corrections du harnais découvertes par la matrice

Le premier run du nouveau test s’est arrêté avant les assertions de cache : le helper commun déduisait le schéma batch v5 uniquement de `requested_delivery_count`, alors que le résultat batch avec `results` est lui aussi v5 sans ce champ. Le helper distingue maintenant les lots via `results` ou `requested_delivery_count`. La sortie rouge est conservée dans `schema-version-helper-red.log`; elle ne signale pas un défaut runtime du cache.

La matrice a ensuite exercé un test de non-consommation du credential dont le premier payload omettait le dépôt. Le receiver l’a rejeté HTTP 400 avant journalisation, conformément à sa validation d’ingestion. Une première correction l’a rendu valide et dans le périmètre, ce qui a fait lire le credential comme prévu. La fixture finale emploie deux dépôts hors périmètre bien formés et confirme zéro lecture d’upstream et credential pipe non consommé. La matrice a aussi révélé une assertion isolée qui attendait encore le schéma simple v2; elle utilise désormais le helper de schéma v4/v5. Les traces des deux passages incomplets et des corrections ciblées sont archivées dans ce dossier; le run final de la matrice est vert.

Ces modifications portent sur les tests processus, le helper de test, le runner et les preuves. Le runtime de production n’a pas changé dans ce lot. Elles rendent vérifiables au processus les refus de batch et les schémas réellement émis.

## Périmètre et limites

- L’API est une fixture locale, pas une instance Forgejo; la matrice n’est pas une qualification du service amont.
- Le test couvre plusieurs IDs pour la même ressource, pas une même livraison rejouée par l’inbox.
- Le cache et ses refus sont observés sur ce batch; aucune réduction de coût globale ou benchmark n’est revendiqué.
- Aucun LLM, AgentOS, credential Forgejo réel, service externe ou production n’a été utilisé.
- Cargo conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected` et `redistribution_rights=not_assessed`; aucune licence n’est attribuée.
- Le parcours LLM et la comparaison contrôlée restent non démontrés. Projet incomplet.

Le HEAD, les versions, commandes, sorties et empreintes sont consignés dans ce dossier. Les sources test/runner sont non suivies dans le worktree au commit HEAD décrit par `toolchain.txt`.
