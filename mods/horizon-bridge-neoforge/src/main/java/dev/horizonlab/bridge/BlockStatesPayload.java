package dev.horizonlab.bridge;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;

/**
 * {@code horizon_bridge:block_states}: every block in registry order with its id, first and default
 * state ids, its properties (values in declaration order) and, per state, the index of each
 * property's value. Read by neoforge.rs {@code read_block_states}.
 */
public record BlockStatesPayload() implements CustomPacketPayload {
    public static final Type<BlockStatesPayload> TYPE = new Type<>(HorizonBridge.id("block_states"));
    public static final StreamCodec<FriendlyByteBuf, BlockStatesPayload> STREAM_CODEC =
            StreamCodec.of((buf, payload) -> write(buf), buf -> {
                throw new UnsupportedOperationException("The block-state table is only sent by the server");
            });

    @Override
    public Type<BlockStatesPayload> type() {
        return TYPE;
    }

    private static void write(FriendlyByteBuf buf) {
        buf.writeVarInt(BuiltInRegistries.BLOCK.size());
        for (Block block : BuiltInRegistries.BLOCK) {
            buf.writeUtf(BuiltInRegistries.BLOCK.getKey(block).toString());
            List<BlockState> states = block.getStateDefinition().getPossibleStates();
            buf.writeVarInt(Block.getId(states.get(0)));
            buf.writeVarInt(Block.getId(block.defaultBlockState()));
            List<Property<?>> properties = new ArrayList<>(block.getStateDefinition().getProperties());
            buf.writeVarInt(properties.size());
            for (Property<?> property : properties) {
                buf.writeUtf(property.getName());
                buf.writeVarInt(property.getPossibleValues().size());
                writeValueNames(buf, property);
            }
            buf.writeVarInt(states.size());
            for (BlockState state : states) {
                for (Property<?> property : properties) {
                    buf.writeByte(new ArrayList<>(property.getPossibleValues()).indexOf(state.getValue(property)));
                }
            }
        }
    }

    private static <T extends Comparable<T>> void writeValueNames(FriendlyByteBuf buf, Property<T> property) {
        for (T value : property.getPossibleValues()) {
            buf.writeUtf(property.getName(value));
        }
    }
}
