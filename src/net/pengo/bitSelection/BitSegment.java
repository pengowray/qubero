/**
 * Segment.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.bitSelection;

public class BitSegment implements Cloneable, Comparable {
    public BitCursor firstIndex;
    public BitCursor lastIndex;
    
    public BitSegment(long index0, long index1) {
        this(new BitCursor(index0,0), new BitCursor(index1,0));
    }
    
    public BitSegment(BitCursor index0, BitCursor index1) {
        BitCursor[] sorted = new BitCursor[] { index0, index1 };
        java.util.Arrays.sort(sorted);
        firstIndex = sorted[0];
        lastIndex = sorted[1];
    }
    
    /** new segment from extremes (min and max) found in array. warning: Sorts the indexes in place */
    public BitSegment(BitCursor[] indexes) {
        //fixme: needs to remove nulls
        
        java.util.Arrays.sort(indexes);
        firstIndex = indexes[0];
        lastIndex = indexes[indexes.length-1];
    }
    
    
    public String toString() {
        return firstIndex + "-" + lastIndex;
    }
    
    public BitSegment clone() {
        //fixme: stop from resorting (minor optimisation)
        //fixme: should clone firstIndex/lastIndex .. or make them immutable.. or make BitSegment immutable too
        return new BitSegment(firstIndex, lastIndex);
    }
    
    /** makes a BitSegment between the extreme ranges of this one and another */
    public BitSegment maxRange(BitSegment merge) {
        if (merge == null)
            return this.clone();
        
        BitCursor[] sorted = new BitCursor[] { firstIndex, lastIndex, merge.firstIndex, merge.lastIndex };
        java.util.Arrays.sort( sorted );
        return new BitSegment(sorted[0], sorted[3]);
    }
    
    public BitCursor getLength() {
        return lastIndex.subtract(firstIndex);
    }
    
    public int compareTo(Object o) {
         if (o instanceof BitSegment)
             return compareTo((BitSegment)o);
         
         if (o instanceof BitCursor)
             return firstIndex.compareTo((BitCursor)o);
         
         throw new NullPointerException("sorry.");
    }
    
    public int compareTo(BitSegment o) {
            return this.firstIndex.compareTo(o.firstIndex);
    }
    
    public int compareTo(BitCursor o) {
            return firstIndex.compareTo(o);
    }
    
}
