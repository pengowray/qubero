/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/**
 * Segment.java
 *
 * @author Peter Halasz
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
        System.out.println("bs:" + this);
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
    
    /** WARNING: only compares first index! */
    public boolean equals(Object o) {
    	if (o instanceof BitSegment)
    		return (compareTo((BitSegment)o) == 0);
    	
    	return super.equals(o);
    	
    }

	public int compareTo(BitSegment o) {
            return this.firstIndex.compareTo(o.firstIndex);
    }
    
    public int compareTo(BitCursor o) {
            return firstIndex.compareTo(o);
    }
    
    /** subtracts from both indexes. if last index would be is below 0 then returns null */ 
    public BitSegment shiftLeft(BitCursor shift) {
    	if (lastIndex.compareTo(shift) < 0)
    		return null;
    	
    	return new BitSegment(firstIndex.subtract(shift), lastIndex.subtract(shift));
    }
    
    /** is this an empty selection? (i.e. is length == 0) */
    public boolean isEmpty() { 
    	if (firstIndex == lastIndex || firstIndex.equals(lastIndex) ) {
    		return true;
    	}
    	
    	return false;
    }
}
