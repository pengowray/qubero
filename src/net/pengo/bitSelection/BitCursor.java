/*
 * BitCursor.java
 *
 * Created on 25 November 2004, 05:30
 */

package net.pengo.bitSelection;

/**
 *
 * @author  Que
 */
public class BitCursor implements Comparable {
    //0 means just before the first byte/bit
    //1 means after the first byte/bit
    private long byteOffset;
    private int bitOffset;
    //public static void NULL_BIT_CURSOR = new BitCursor {}
    
    /** Creates a new instance of BitCursor */
    public BitCursor() {
        byteOffset = 0;
        bitOffset = 0;
        normalize();
    }
    
    public static BitCursor newFromBits(long bitOffset) {
        // bit normalization
        BitCursor ret = new BitCursor();
        if (bitOffset > 7 || bitOffset < 0) {
            ret.byteOffset = ret.byteOffset + (bitOffset / 8);
            ret.bitOffset = (int)(bitOffset % 8);
        }
        ret.normalize();
        
        return ret;
    }
    
    public BitCursor(long byteOffset, int bitOffset) {
        this.byteOffset = byteOffset ;
        this.bitOffset = bitOffset;
    }
    
    /**
     * make sure the bit offset is 0-7. and bitoffset >= 0
     */
    private void normalize() {
        
        // bit normalization
        if (bitOffset > 7 || bitOffset < 0) {
            byteOffset = byteOffset + (bitOffset / 8);
            bitOffset %= 8;
        }
        
        // check byteOffset >= 0. 
        // must be done after bit normalization
        if (byteOffset < 0) {
            byteOffset = 0;
            bitOffset = 0;
        }
    }
    
    public long getByteOffset() {
        return byteOffset;
    }
    
    public void setByteOffset(long byteOffset) {
        this.byteOffset = byteOffset;
        normalize();
    }

    public int getBitOffset() {
        return bitOffset;
    }

    public void setBitOffset(int bitOffset) {
        this.bitOffset = bitOffset;
        normalize();
    }
    
    /** ugh. use static newFromBits() instead */
    /*
    public void addBits(long bits) {
        //fixme: somewhat inefficient
        this.bitOffset = bitOffset + (int)(bits % 8);
        this.byteOffset = byteOffset + (bits / 8);
        normalize();
    }
     */

    public int compareTo(Object o) {
        if (o instanceof BitCursor)
            return compareTo((BitCursor)o);
        
        if (o instanceof DirectionalSegment)
            return compareTo((DirectionalSegment)o);
        
        //fixme: lazy
        return -1;
    }
    
    public int compareTo(DirectionalSegment o) {
        return -o.compareTo(this);
    }

    public int compareTo(BitCursor o) {
        if (this.byteOffset < o.byteOffset)
            return -1;
        
        if (this.byteOffset > o.byteOffset)
            return 1;
        
        if (this.bitOffset < o.bitOffset)
            return -1;
        
        if (this.bitOffset > o.bitOffset)
            return 1;
        
        return 0;
    }
    
    public boolean equals(BitCursor o) {
        if (this.byteOffset == o.byteOffset && this.bitOffset == o.bitOffset)
            return true;
        
        return false;
    }
    
    public String toString() {
        if (bitOffset == 0)
            return Long.toHexString( byteOffset );
            
        return Long.toHexString( byteOffset ) + "." + bitOffset;
    }
    
    /** value as bits */
    public long toBits() {
        return byteOffset * 8 + bitOffset;
    }
    
    public BitCursor addBits(int bits) {
        return new BitCursor(byteOffset, bitOffset+bits);
    }
    
    /** subtract amount from this cursor and return the result as a new bitcursor.
      * note! there are no negative cursors! a negative result will give 0 
      */
    public BitCursor subtract(BitCursor amount) {
        return new BitCursor(this.byteOffset - amount.byteOffset, this.bitOffset - amount.bitOffset );
    }

    public BitCursor add(BitCursor amount) {
        return new BitCursor(this.byteOffset + amount.byteOffset, this.bitOffset + amount.bitOffset );
    }
    
    public long divide(BitCursor divideBy) {
        //fixme.. ?
        return this.toBits() / divideBy.toBits();
    }
    
    //public BitCursor multiply(BitCursor multiplyBy) {
        // fixme: this is too dodgy. maybe need to cross multiply or something
        
        //return new BitCursor(byteOffset * multiplyBy.byteOffset, bitOffset * multiplyBy.bitOffset); // 0.2 x 2.0 = 0.0
    //}
    
    public BitCursor multiply(int multiplyBy) {
        return new BitCursor(byteOffset * multiplyBy, bitOffset * multiplyBy);
    }

    public BitCursor mod(BitCursor mod) {
        return BitCursor.newFromBits( this.toBits() % mod.toBits() );;
    }
}
