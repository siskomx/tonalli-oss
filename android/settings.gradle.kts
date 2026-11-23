pluginManagement {
    repositories { google(); mavenCentral(); gradlePluginPortal() }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories { google(); mavenCentral() }
}
rootProject.name = "TonalliAndroidSpike"
include(":app")
if (providers.gradleProperty("meshSpike").orNull == "true") include(":mesh-spike")
