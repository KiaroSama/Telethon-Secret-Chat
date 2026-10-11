"""Effect checkpoints around the existing caller-owned client, never a new client."""

import asyncio


class AdmittedClient:
    def __init__(self, owner, client):
        self.owner, self.client = owner, client

    def __getattr__(self, name):
        return getattr(self.client, name)

    async def __call__(self, request):
        chat_id = getattr(getattr(request, "peer", None), "chat_id", None)
        if chat_id is not None:
            self.owner._admit(chat_id, "rpc")
        result = await self.client(request)
        if chat_id is not None and type(request).__name__ == "AcceptEncryptionRequest":
            # Its persisted DH secret can reconcile an uncertain accepted request;
            # do not install a new key or emit ready after an operator pause.
            self.owner._admit(chat_id, "accept-result")
        return result

    async def upload_file(self, source, **kwargs):
        owner = self.owner
        task = asyncio.current_task()
        chat_id = owner._active.get(task)
        owner._admit(chat_id, "upload")

        class CheckedReader:
            def __getattr__(self, name):
                return getattr(source, name)

            def read(self, count=-1):
                owner._admit(chat_id, "upload-chunk")
                return source.read(count)

        result = await self.client.upload_file(CheckedReader(), **kwargs)
        owner._admit(chat_id, "upload-result")
        return result
