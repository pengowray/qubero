/*
 * based on SimpleLongListSelectionModel.java
 *
 * Created on 19 November 2002, 10:09
 */

/**
 * Simple selection between two points. Cannot be edited.
 *
 * @author  Noam Chomsky
 */
package net.pengo.resource;


import net.pengo.pointer.JavaPointer;
import net.pengo.selection.LongListSelectionModel;
import net.pengo.app.OpenFile;
import net.pengo.data.SelectionData;
import net.pengo.selection.LongListSelectionListener;
import net.pengo.selection.Segment;
import net.pengo.selection.SegmentalLongListSelectionModel;
import net.pengo.selection.SimpleLongListSelectionModel;

public class SimpleSelectionResource extends SelectionResource implements LongListSelectionModel {
    public final JavaPointer firstIndexP = new JavaPointer(IntResource.class);
    public final JavaPointer lastIndexP = new JavaPointer(IntResource.class);
    //fixme: make a codependant length attribute
    
    /** Creates a new instance of SimpleLongListSelectionModel */
    public SimpleSelectionResource(OpenFile openFile, long firstIndex, long lastIndex) {
	super(openFile);
	firstIndexP.setName("First index");
	firstIndexP.addSink(this);
	
	lastIndexP.setName("Last index");
	lastIndexP.addSink(this);
	
	setFirstIndex(firstIndex);
	setLastIndex(lastIndex);
    }
    
    public SimpleSelectionResource(SelectionResource selRes) {
	//fixme: may turn multi-selection into a simple selection, but what else can you do?
	this(selRes.getOpenFile(), selRes.getSelection().getMinSelectionIndex(), selRes.getSelection().getMaxSelectionIndex());
    }

    public String valueDesc() {
	//fixme ?
	return "" + getFirstIndex() + "-" + getLastIndex();
    } 
    
    public Resource[] getSources() {
	return new Resource[] {firstIndexP, lastIndexP};
    }
    
    public void setSelection(LongListSelectionModel sel) {
	System.out.println("setSelection() not supported by " + this.getClass() + " (class) in " + this);
    }
    
    public LongListSelectionModel getSelection() {
	return this;
    }
    
    public SelectionData getSelectionData() {
	return new SelectionData(getSelection(), getOpenFile().getData());
    }
    
    public void setFirstIndex(long index) {
	firstIndexP.setValue(new IntPrimativeResource(index));
    }
    
    public long getFirstIndex() {
	return ((IntResource)(firstIndexP.evaluate())).toLong();
    }
    
    public void setLastIndex(long index) {
	lastIndexP.setValue(new IntPrimativeResource(index));
    }
    
    public long getLastIndex() {
	return ((IntResource)(lastIndexP.evaluate())).toLong();
    }
    
    public SegmentalLongListSelectionModel toSegmental() {
	SegmentalLongListSelectionModel segmental = new SegmentalLongListSelectionModel();
	segmental.addSelectionInterval(getFirstIndex(), getLastIndex());
	return segmental;
    }
    
    public long getSegmentCount() {
	return 1;
    }
    
    public Segment[] getSegments() {
	return new Segment[] { new Segment(getFirstIndex(), getLastIndex()) };
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
	return getFirstIndex();
    }
    
    public long getLeadSelectionIndex() {
	return getLastIndex();
    }
    
    public long getMaxSelectionIndex() {
	return getLastIndex();
    }
    
    public long getMinSelectionIndex() {
	return getFirstIndex();
    }
    
    public long[] getSelectionIndexes() {
	long length = getLastIndex() - getFirstIndex() + 1;
	
	if (length <= 0) {
	    return new long[0];
	}
	
	long[] indexes = new long[(int)length]; //FIXME: possible loss of precision. always a prob.
	for (int i = 0; i < length; i++ ) {
	    indexes[i] = getFirstIndex() + i;
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
	if (index <= getLastIndex() && index >= getFirstIndex())
	    return true;
	
	return false;
    }
    
    public boolean isSelectionEmpty() {
	if (getFirstIndex() == -1 && getLastIndex() == -1)
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
	
	if (start+length > (getLastIndex()-getFirstIndex()) ) {
	    // trim tail
	    length = getLastIndex()-getFirstIndex();
	}
	
	return new SimpleLongListSelectionModel(getFirstIndex()+start, getFirstIndex()+start+length);
    }
    
    public void updated(){
	// do nothing.
    }
}
