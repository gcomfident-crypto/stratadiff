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

downloads_directory=$cache_directory/downloads
if [[ -e $downloads_directory || -L $downloads_directory ]]; then
  [[ -d $downloads_directory && ! -L $downloads_directory ]] || {
    echo "cache downloads path is not a directory: $downloads_directory" >&2
    exit 1
  }
else
  mkdir -p "$downloads_directory"
fi

fetch_verified() {
  local url=$1
  local expected=$2
  local destination=$3
  if [[ -e $destination || -L $destination ]]; then
    [[ -f $destination && ! -L $destination ]] || {
      echo "cached artifact is not a regular file: $destination" >&2
      exit 1
    }
  else
    local partial=${destination}.part
    if [[ -e $partial || -L $partial ]]; then
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

prepare_partial() {
  local partial=$1
  if [[ -e $partial || -L $partial ]]; then
    [[ -f $partial && ! -L $partial ]] || {
      echo "partial output is not a regular file: $partial" >&2
      exit 1
    }
    rm "$partial"
  fi
}

require_regular_destination() {
  local destination=$1
  if [[ -e $destination || -L $destination ]]; then
    [[ -f $destination && ! -L $destination ]] || {
      echo "cache output is not a regular file: $destination" >&2
      exit 1
    }
  fi
}

gumtree_zip=$cache_directory/downloads/gumtree-3.0.0.zip
jdt_jar=$cache_directory/downloads/org.eclipse.jdt.core-3.35.0.jar
ecj_jar=$cache_directory/downloads/ecj-3.35.0.jar
fetch_verified "$gumtree_url" "$gumtree_sha256" "$gumtree_zip"
fetch_verified "$jdt_url" "$jdt_sha256" "$jdt_jar"
fetch_verified "$ecj_url" "$ecj_sha256" "$ecj_jar"

gumtree_fat_jar=$cache_directory/downloads/gumtree-3.0.0.jar
if [[ ! -e $gumtree_fat_jar ]]; then
  gumtree_fat_jar_part=${gumtree_fat_jar}.part
  if [[ -e $gumtree_fat_jar_part || -L $gumtree_fat_jar_part ]]; then
    [[ -f $gumtree_fat_jar_part && ! -L $gumtree_fat_jar_part ]] || {
      echo "partial GumTree JAR is not a regular file: $gumtree_fat_jar_part" >&2
      exit 1
    }
    rm "$gumtree_fat_jar_part"
  fi
  unzip -p "$gumtree_zip" gumtree-3.0.0/lib/gumtree.jar > "$gumtree_fat_jar_part"
  printf '%s  %s\n' "$gumtree_jar_sha256" "$gumtree_fat_jar_part" | sha256sum --check --status
  mv "$gumtree_fat_jar_part" "$gumtree_fat_jar"
fi
[[ -f $gumtree_fat_jar && ! -L $gumtree_fat_jar ]] || {
  echo "cached GumTree JAR is not a regular file: $gumtree_fat_jar" >&2
  exit 1
}
printf '%s  %s\n' "$gumtree_jar_sha256" "$gumtree_fat_jar" | sha256sum --check --status || {
  echo "cached GumTree JAR digest mismatch: $gumtree_fat_jar" >&2
  exit 1
}

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
javac_command=$(realpath "$javac_command")
java_command=$(realpath "$java_command")
javac_version=$($javac_command -version 2>&1)
javac_major=${javac_version#javac }
javac_major=${javac_major%%.*}
if [[ ! $javac_major =~ ^[0-9]+$ || $javac_major -lt 17 ]]; then
  echo "Java 17 or newer JDK is required, found: $javac_version" >&2
  exit 1
fi
java_version=$($java_command -version 2>&1)
java_major=$(
  "$java_command" -XshowSettings:properties -version 2>&1 |
    sed -n 's/^[[:space:]]*java.version = \([0-9][0-9]*\).*/\1/p'
)
if [[ ! $java_major =~ ^[0-9]+$ || $java_major -lt 17 ]]; then
  echo "Java 17 or newer runtime is required, found: ${java_version%%$'\n'*}" >&2
  exit 1
fi

provenance_directory=$cache_directory/provenance
if [[ -e $provenance_directory ]]; then
  [[ -d $provenance_directory && ! -L $provenance_directory ]] || {
    echo "cache provenance path is not a directory: $provenance_directory" >&2
    exit 1
  }
else
  mkdir "$provenance_directory"
fi
helper_neutral_source=$provenance_directory/EnumerateJdt.java.source
helper_source_sha256=$(sha256sum "$script_directory/EnumerateJdt.java.source")
helper_source_sha256=${helper_source_sha256%% *}
require_regular_destination "$helper_neutral_source"
prepare_partial "${helper_neutral_source}.part"
cp "$script_directory/EnumerateJdt.java.source" "${helper_neutral_source}.part"
mv "${helper_neutral_source}.part" "$helper_neutral_source"
printf '%s  %s\n' "$helper_source_sha256" "$helper_neutral_source" |
  sha256sum --check --status
require_regular_destination "$provenance_directory/helper-source.sha256"
prepare_partial "$provenance_directory/helper-source.sha256.part"
printf '%s\n' "$helper_source_sha256" > "$provenance_directory/helper-source.sha256.part"
mv "$provenance_directory/helper-source.sha256.part" \
  "$provenance_directory/helper-source.sha256"
require_regular_destination "$provenance_directory/java-executable"
prepare_partial "$provenance_directory/java-executable.part"
printf '%s\n' "$java_command" > "$provenance_directory/java-executable.part"
mv "$provenance_directory/java-executable.part" "$provenance_directory/java-executable"
require_regular_destination "$provenance_directory/java-version"
prepare_partial "$provenance_directory/java-version.part"
printf '%s\n' "$java_version" > "$provenance_directory/java-version.part"
mv "$provenance_directory/java-version.part" "$provenance_directory/java-version"

helper_classes=$cache_directory/helper
if [[ -e $helper_classes ]]; then
  [[ -d $helper_classes && ! -L $helper_classes ]] || {
    echo "helper class path is not a directory: $helper_classes" >&2
    exit 1
  }
else
  mkdir "$helper_classes"
fi
helper_source_directory=$cache_directory/helper-source
if [[ -e $helper_source_directory ]]; then
  [[ -d $helper_source_directory && ! -L $helper_source_directory ]] || {
    echo "helper source path is not a directory: $helper_source_directory" >&2
    exit 1
  }
else
  mkdir "$helper_source_directory"
fi
helper_java=$helper_source_directory/EnumerateJdt.java
require_regular_destination "$helper_java"
prepare_partial "${helper_java}.part"
cp "$helper_neutral_source" "${helper_java}.part"
mv "${helper_java}.part" "$helper_java"
printf '%s  %s\n' "$helper_source_sha256" "$helper_java" |
  sha256sum --check --status
classpath=$jdt_jar:$ecj_jar:$gumtree_fat_jar
if [[ -e $helper_classes/EnumerateJdt.class || -L $helper_classes/EnumerateJdt.class ]]; then
  [[ -f $helper_classes/EnumerateJdt.class && ! -L $helper_classes/EnumerateJdt.class ]] || {
    echo "cached helper class is not a regular file: $helper_classes/EnumerateJdt.class" >&2
    exit 1
  }
fi
"$javac_command" -proc:none --release 17 -classpath "$classpath" \
  -d "$helper_classes" "$helper_java"

launcher=$cache_directory/enumerate-jdt
launcher_part=${launcher}.part
require_regular_destination "$launcher"
prepare_partial "$launcher_part"
printf '#!/usr/bin/env bash\nexec %q -classpath %q EnumerateJdt "$@"\n' \
  "$java_command" "$helper_classes:$classpath" > "$launcher_part"
chmod 0755 "$launcher_part"
mv "$launcher_part" "$launcher"
printf '%s\n' "$launcher"
