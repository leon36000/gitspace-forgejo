# Archive textuel des preuves PC6

Le dépôt contient les sources, tests et documents actifs séparément. Cette archive conserve les preuves texte historiques. Les blobs Base64 sont découpés en morceaux de 256 000 caractères et reconstruits localement avant le transfert.

Depuis la racine du dépôt, reconstruire et vérifier :

    cat evidence/gitlab-pc4-transfer-002/payload-*.b64 | base64 -d > /tmp/forge-evidence.tar.gz
    sha256sum -c evidence/gitlab-pc4-transfer-002/archive.sha256
    tar -xzf /tmp/forge-evidence.tar.gz -C .
    python3 - <<'PYVERIFY'
import hashlib,json,pathlib,sys
rows=[json.loads(x) for x in pathlib.Path('evidence/gitlab-pc4-transfer-002/CONTENTS.jsonl').read_text().splitlines()]
bad=[]
for row in rows:
 p=pathlib.Path(row['path'])
 if not p.is_file() or p.stat().st_size!=row['size'] or hashlib.sha256(p.read_bytes()).hexdigest()!=row['sha256']: bad.append(row['path'])
if bad: print('mismatch_count',len(bad)); sys.exit(1)
print('verified_files',len(rows))
PYVERIFY

L’archive exclut l’état .runtime/.claude, les caches, les snapshots ZIP binaires, les objets Git embarqués, et les fichiers Go issus des POC Forgejo. La licence de GitSpace demeure indéterminée; aucun code GitSpace n’est inclus. Voir bundle.json et le checkpoint pour les limites.
