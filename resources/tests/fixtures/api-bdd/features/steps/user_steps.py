"""Sample API step definitions used by sidecar tests and the fixture project."""

from teshi_api import call, when


@when("I create a user named {name}")
def create_user(context, name):  # noqa: ARG001
    call("create_user.json.j2")


@when("I fetch that user")
def fetch_user(context):  # noqa: ARG001
    call("get_user.json.j2")
