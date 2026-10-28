# FORGEJO-SEMANTIC-CLEAN-MERGE-LIVE-001 — refus puis correction sur une PR locale

Date : 26 octobre 2026 (America/Montreal). Worktree produit fortement modifié; base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`. Aucun reset, commit ou changement de production. L’état privé Forgejo reste conservé sous `/tmp/forgejo-semantic-clean-merge-live-003`.

## Parcours observé

Une instance locale Forgejo 17.0.0 a reçu deux PR issues de branches créées sur le même commit de fixture. Chaque vérification propre à sa branche passe. Le gate indépendant a approuvé puis fusionné la PR de borne supérieure. Forgejo a ensuite traité `POST /pulls/5/update` (HTTP 200) pour intégrer la nouvelle base dans la PR de borne inférieure. Son nouveau head `988543381714e06aefb6aa4788997b83cd8a3299` est mergeable; le SHA du fichier lu depuis ce head égale exactement celui de l’union attendue des deux sources.

Les tests indépendants inférieur et supérieur restent verts sur cet arbre combiné. Le critère protégé, conservé côté vérificateur, demande `Err(EmptyRange)` quand les deux bornes exclusives rendent `[0,1]` vide. Il échoue sur `min > max` (code 101). Le proof lié à la base `9cd64c9063feceaec6b0545133ff9ebc3f5c5b6d`, au candidate `988543381714e06aefb6aa4788997b83cd8a3299`, aux exigences et à la politique est refusé par l’autorité réelle : `verdict=rejected`, `error.code=verification_failed`, `merge_attempted=false`.

Le premier runner s’est arrêté avec code 1 parce que son assertion attendait un autre nom public d’erreur. Le résultat live du gate était le refus attendu. L’instance et son état ont été conservés. Le processus Forgejo a été arrêté puis redémarré sur cette base; le script de reprise n’a continué qu’après avoir vérifié que le PR était ouvert, que `main` correspondait toujours à la base du preview et que le head n’avait pas changé. Il a corrigé la borne vide, exécuté le critère protégé, signé une nouvelle preuve et demandé une revue distincte. Le gate a fusionné le head corrigé `1c3d7f0a0c014a9fa88e2009f6cab35eaa96a68c` en `42f48189560607ec2c391e50526f5e5560cfb524`. La PR et l’issue sont fermées; le contenu relu depuis `main` est identique à celui accepté et le même critère passe après fusion.

## Commandes et environnement

Les commandes de démarrage et de reprise exactes sont conservées dans `commands.txt`; l’historique des deux assertions de runner est dans `iteration-history.md`. La reprise a réutilisé la même base Forgejo et les journaux déjà produits; elle n’a pas réexécuté les vérifications rouges/vertes antérieures.

Forgejo : `17.0.0+gitea-1.22.0`, binaire local SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit exact de build de ce binaire n’est pas établi. Les détails des outils figurent dans `toolchain.txt`; les empreintes des sources et fixtures sont dans `source-hashes.json`.

Vérifications fraîches après les changements du runner et de la reprise : `python3 -m unittest discover -s tests -p 'test_*.py' -q` passe 251/251; `python3 -m unittest discover -s scripts -p 'test_*.py' -q` passe 10/10. `py_compile` des trois scripts et deux tests concernés, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent. Pytest n’est pas installé (`No module named pytest`); aucun paquet n’a été ajouté. Les manifestes sont rafraîchis après cette entrée.

## Limites

Ce résultat est une instance et une fixture déterministe (`n=1`), pas un parcours LLM/AgentOS, ni une CI conventionnelle. Il ne mesure pas tokens, coût, interventions ou productivité et ne démontre aucun gain général. La preuve de rejet et la reprise ont été achevées sur la même base persistée à travers un redémarrage, mais le lancement complet du runner n’a pas été rejoué après correction de ses assertions. Le test de preuve périmée après changement est couvert par le parcours standard antérieur; ce canari n’affirme pas l’avoir re-démontré avec un proof en outcome `pass`. La limite reste un verrou coopératif : les writers externes Forgejo ne sont pas exclus. La frontière locale ne démontre ni isolation multi-tenant ni résistance à un noyau compromis. Aucun code Forgejo/GitSpace n’est copié; aucun LLM, service externe ou secret utilisateur n’est utilisé.

La licence produit reste non sélectionnée (`license=null`, `publish=[]`, droits non évalués). L’inventaire GitSpace/licences, l’intégration AgentOS/runtime autorisée et la comparaison appariée restent ouverts; la mission n’est pas complète.
