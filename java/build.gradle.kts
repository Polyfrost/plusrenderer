plugins {
    `java-library`
    `maven-publish`
}

java {
    toolchain.languageVersion = JavaLanguageVersion.of(8)
    withSourcesJar()
}

// scripts/build-native.sh writes natives/plusrenderer/<os>-<arch>/ here, and release CI fills in every platform.
sourceSets.main { resources.srcDir("../target/java-resources") }

repositories { mavenCentral() }

dependencies { testImplementation("junit:junit:4.13.2") }

publishing {
    publications.create<MavenPublication>("maven") { from(components["java"]) }
    // -PreposiliteUrl=... plus reposiliteUsername / reposilitePassword (or ORG_GRADLE_PROJECT_* env vars).
    providers.gradleProperty("reposiliteUrl").orNull?.takeIf(String::isNotEmpty)?.let { target ->
        repositories.maven {
            name = "reposilite"
            url = uri(target)
            credentials(PasswordCredentials::class)
        }
    }
}
