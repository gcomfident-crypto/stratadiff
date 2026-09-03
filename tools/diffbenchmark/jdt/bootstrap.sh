#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 ABSOLUTE_CACHE_DIRECTORY" >&2
  exit 2
fi

cache_directory=$1
case $cache_directory in
  /*) ;;
  *) echo "cache directory must be absolute: $cache_directory" >&2; exit 2 ;;
esac
if [[ $cache_directory == / ]]; then
  echo "refusing to use / as the cache directory" >&2
  exit 2
fi
if [[ -L $cache_directory ]]; then
  echo "cache directory must not be a symbolic link: $cache_directory" >&2
  exit 2
fi

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
gumtree_url=https://github.com/GumTreeDiff/gumtree/releases/download/v3.0.0/gumtree-3.0.0.zip
gumtree_sha256=9c9672be9310d910f23e0130484cd46e407d1e6530e3fba0546346d15de465a7
gumtree_jar_sha256=959404693f963f658ff2c6a9111eca9fa392a845ce656613178c96994515f909
jdt_url=https://repo.maven.apache.org/maven2/org/eclipse/jdt/org.eclipse.jdt.core/3.35.0/org.eclipse.jdt.core-3.35.0.jar
jdt_sha256=8f5bcb00355b271638b9d82a8cccd0e733225cb74c4a3f0f55a7b75d43ee442a
ecj_url=https://repo.maven.apache.org/maven2/org/eclipse/jdt/ecj/3.35.0/ecj-3.35.0.jar
ecj_sha256=b89df382369f2d16b19b67085426dc0fb7401fe1ad9fba1806af4e9729f5d1c0

mkdir -p "$cache_directory/downloads"

fetch_verified() {
  local url=$1
  local expected=$2
  local destination=$3
  if [[ -e $destination ]]; then
    [[ -f $destination && ! -L $destination ]] || {
      echo "cached artifact is not a regular file: $destination" >&2
      exit 1
    }
  else
    local partial=${destination}.part
    if [[ -e $partial ]]; then
      [[ -f $partial && ! -L $partial ]] || {
        echo "partial artifact is not a regular file: $partial" >&2
        exit 1
      }
      rm "$partial"
    fi
    curl --fail --location --silent --show-error "$url" --output "$partial"
    printf '%s  %s\n' "$expected" "$partial" | sha256sum --check --status
    mv "$partial" "$destination"
  fi
  printf '%s  %s\n' "$expected" "$destination" | sha256sum --check --status || {
    echo "cached artifact digest mismatch: $destination" >&2
    exit 1
  }
}

gumtree_zip=$cache_directory/downloads/gumtree-3.0.0.zip
jdt_jar=$cache_directory/downloads/org.eclipse.jdt.core-3.35.0.jar
ecj_jar=$cache_directory/downloads/ecj-3.35.0.jar
fetch_verified "$gumtree_url" "$gumtree_sha256" "$gumtree_zip"
fetch_verified "$jdt_url" "$jdt_sha256" "$jdt_jar"
fetch_verified "$ecj_url" "$ecj_sha256" "$ecj_jar"

gumtree_fat_jar=$cache_directory/downloads/gumtree-3.0.0.jar
if [[ ! -e $gumtree_fat_jar ]]; then
  unzip -p "$gumtree_zip" gumtree-3.0.0/lib/gumtree.jar > "${gumtree_fat_jar}.part"
  printf '%s  %s\n' "$gumtree_jar_sha256" "${gumtree_fat_jar}.part" | sha256sum --check --status
  mv "${gumtree_fat_jar}.part" "$gumtree_fat_jar"
fi
printf '%s  %s\n' "$gumtree_jar_sha256" "$gumtree_fat_jar" | sha256sum --check --status || {
  echo "cached GumTree JAR digest mismatch: $gumtree_fat_jar" >&2
  exit 1
}

gumtree_classes=$cache_directory/gumtree-3.0.0-without-jdt
if [[ ! -d $gumtree_classes ]]; then
  staging=$cache_directory/.gumtree-classes-staging
  [[ ! -e $staging ]] || {
    echo "stale GumTree staging path must be removed manually: $staging" >&2
    exit 1
  }
  mkdir "$staging"
  unzip -q "$gumtree_fat_jar" -d "$staging"
  if [[ -d $staging/org/eclipse/jdt ]]; then
    find "$staging/org/eclipse/jdt" -depth -delete
  fi
  find "$staging/META-INF" -maxdepth 1 -type f \
    \( -name '*.SF' -o -name '*.RSA' -o -name '*.DSA' \) -delete
  mv "$staging" "$gumtree_classes"
fi
if [[ -e $gumtree_classes/org/eclipse/jdt ]]; then
  echo "GumTree cache still contains its embedded JDT 3.26 classes" >&2
  exit 1
fi

if [[ -n ${JAVA_HOME-} ]]; then
  javac_command=$JAVA_HOME/bin/javac
  java_command=$JAVA_HOME/bin/java
else
  javac_command=$(command -v javac || true)
  java_command=$(command -v java || true)
fi
if [[ ! -x $javac_command || ! -x $java_command ]]; then
  echo "Java 17 or newer JDK is required (set JAVA_HOME)" >&2
  exit 1
fi
javac_version=$($javac_command -version 2>&1)
javac_major=${javac_version#javac }
javac_major=${javac_major%%.*}
if [[ ! $javac_major =~ ^[0-9]+$ || $javac_major -lt 17 ]]; then
  echo "Java 17 or newer JDK is required, found: $javac_version" >&2
  exit 1
fi

helper_classes=$cache_directory/helper
mkdir -p "$helper_classes"
classpath=$jdt_jar:$ecj_jar:$gumtree_classes
"$javac_command" --release 17 -classpath "$classpath" \
  -d "$helper_classes" "$script_directory/EnumerateJdt.java"

launcher=$cache_directory/enumerate-jdt
launcher_part=${launcher}.part
printf '#!/usr/bin/env bash\nexec %q -classpath %q EnumerateJdt "$@"\n' \
  "$java_command" "$helper_classes:$classpath" > "$launcher_part"
chmod 0755 "$launcher_part"
mv "$launcher_part" "$launcher"
printf '%s\n' "$launcher"
