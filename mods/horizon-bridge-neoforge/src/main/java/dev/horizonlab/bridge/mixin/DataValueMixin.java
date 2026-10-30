package dev.horizonlab.bridge.mixin;

import dev.horizonlab.bridge.BridgeFraming;
import io.netty.buffer.Unpooled;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.syncher.EntityDataSerializer;
import net.minecraft.network.syncher.EntityDataSerializers;
import net.minecraft.network.syncher.SynchedEntityData;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** For the bridge, each entity data value goes out with its length in front. */
@Mixin(SynchedEntityData.DataValue.class)
public abstract class DataValueMixin<T> {
    @Shadow @Final private int id;
    @Shadow @Final private EntityDataSerializer<T> serializer;
    @Shadow @Final private T value;

    @Inject(method = "write", at = @At("HEAD"), cancellable = true)
    private void horizonBridge$delimit(RegistryFriendlyByteBuf buf, CallbackInfo ci) {
        if (!BridgeFraming.active()) {
            return;
        }
        buf.writeByte(id);
        buf.writeVarInt(EntityDataSerializers.getSerializedId(serializer));
        RegistryFriendlyByteBuf body = new RegistryFriendlyByteBuf(Unpooled.buffer(), buf.registryAccess(), buf.getConnectionType());
        try {
            serializer.codec().encode(body, value);
            buf.writeVarInt(body.readableBytes());
            buf.writeBytes(body);
        } finally {
            body.release();
        }
        ci.cancel();
    }
}
