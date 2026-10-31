package dev.horizonlab.bridge;

import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.entity.EntityDimensions;
import net.minecraft.world.entity.EntityType;

/**
 * {@code horizon_bridge:entity_types}: every entity type a mod adds (not {@code minecraft:}), with
 * its id, width, height and eye height, so the client can spawn and size it. Read by neoforge.rs
 * {@code read_entity_types}.
 */
public record EntityTypesPayload() implements CustomPacketPayload {
    public static final Type<EntityTypesPayload> TYPE = new Type<>(HorizonBridge.id("entity_types"));
    public static final StreamCodec<FriendlyByteBuf, EntityTypesPayload> STREAM_CODEC =
            StreamCodec.of((buf, payload) -> write(buf), buf -> {
                throw new UnsupportedOperationException("The entity types are only sent by the server");
            });

    @Override
    public Type<EntityTypesPayload> type() {
        return TYPE;
    }

    private static void write(FriendlyByteBuf buf) {
        var modded = BuiltInRegistries.ENTITY_TYPE.stream()
                .filter(t -> !BuiltInRegistries.ENTITY_TYPE.getKey(t).getNamespace().equals(ResourceLocation.DEFAULT_NAMESPACE))
                .toList();
        buf.writeVarInt(modded.size());
        for (EntityType<?> type : modded) {
            EntityDimensions size = type.getDimensions();
            buf.writeUtf(BuiltInRegistries.ENTITY_TYPE.getKey(type).toString());
            buf.writeFloat(size.width());
            buf.writeFloat(size.height());
            buf.writeFloat(size.eyeHeight());
        }
    }
}
