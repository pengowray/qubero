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

/*
* Based on SimpleLongListSelectionModel
  * SimpleLongListSelectionModel.java
 *
 * Created on 19 November 2002, 10:09
 */

/**
 * Simple selection between two points. Cannot be edited.
 *
 * @author Peter Halasz
 */
package net.pengo.selection;


import javax.swing.event.EventListenerList;

public class SimpleLongListSelectionModel implements LongListSelectionModel {
    private long firstIndex;
    private long lastIndex;
    
    /** Creates a new instance of SimpleLongListSelectionModel */
    public SimpleLongListSelectionModel(long firstIndex, long lastIndex) {
        this.firstIndex = firstIndex;
        this.lastIndex = lastIndex;
    }
    
    public SegmentalLongListSelectionModel toSegmental() {
	SegmentalLongListSelectionModel segmental = new SegmentalLongListSelectionModel();
	segmental.addSelectionInterval(firstIndex, lastIndex);
	return segmental;
    }
    
    public long getSegmentCount() {
	return 1;
    }
    
    public Segment[] getSegments() {
	return new Segment[] { new Segment(firstIndex, lastIndex) };
    }
    
    
    public void setValueIsAdjusting(boolean valueIsAdjusting) {
        // not allowed
        return;
    }
    
    public void addLongListSelectionListener(LongListSelectionListener x) {
        // no changes will be made
        return;
    }
    
    public void addSelectionInterval(long index0, long index1) {
        // FIXME: throw exception?
        return;
    }
    
    public void clearSelection() {
        //FIXME: maybe allowed
        return;
    }
    
    public long getAnchorSelectionIndex() {
        return firstIndex;
    }
    
    public long getLeadSelectionIndex() {
        return lastIndex;
    }
    
    public long getMaxSelectionIndex() {
        return lastIndex;
    }
    
    public long getMinSelectionIndex() {
        return firstIndex;
    }
    
    public long[] getSelectionIndexes() {
        long length = lastIndex - firstIndex + 1;
        
        if (length <= 0) {
        	return new long[0];
        }
        
        long[] indexes = new long[(int)length]; //FIXME: possible loss of precision. always a prob.
        for (int i = 0; i < length; i++ ) {
	    indexes[i] = firstIndex + i;
        }
        
        return indexes;
    }
    
    public int getSelectionMode() {
        return SINGLE_INTERVAL_SELECTION;
    }
    
    public boolean getValueIsAdjusting() {
        return false;
    }
    
    public void insertIndexInterval(long index, long length, boolean before) {
        //FIXME: should be allowed?
        return;
    }
    
    public boolean isSelectedIndex(long index) {
        if (index <= lastIndex && index >= firstIndex)
	    return true;
        
        return false;
    }
    
    public boolean isSelectionEmpty() {
        if (firstIndex == -1 && lastIndex == -1)
	    return true;
        
        return false;
    }
    
    public void removeIndexInterval(long index0, long index1) {
        //FIXME: should be allowed?
        return;
    }
    
    public void removeLongListSelectionListener(LongListSelectionListener x) {
        return;
    }
    
    public void removeSelectionInterval(long index0, long index1) {
        return;
    }
    
    public void setAnchorSelectionIndex(long index) {
        // not allowed?
        return;
    }
    
    public void setLeadSelectionIndex(long index) {
        // not allowed?
        return;
    }
    
    public void setSelectionInterval(long index0, long index1) {
        // not allowed?
        return;
    }
    
    public void setSelectionMode(int selectionMode) {
        return;
    }
    
    public Object clone() {
	try {
	    return super.clone();
	} catch (CloneNotSupportedException e) {
	    //FIXME: ho hum
	    e.printStackTrace();
	    return null;
	}
    }

	public LongListSelectionModel slice(long start, long length) {
		if (start < 0) {
			// trim pre 0
			length = length + start; //note: start is negative
			start = 0;
		}
		
		if (start+length > (lastIndex-firstIndex) ) {
			// trim tail
			length = lastIndex-firstIndex;
		}
			
		return new SimpleLongListSelectionModel(firstIndex+start, firstIndex+start+length);
	}
}
