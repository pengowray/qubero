/**
 * QNodeResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import java.util.HashSet;
import java.util.Set;
import net.pengo.app.OpenFile;
import net.pengo.dependency.QNode;

// should this be rolled into Resource ?

abstract public class QNodeResource extends Resource implements QNode {
    
    private Set sinkSet = new HashSet();
    
    public QNodeResource(OpenFile openFile) {
	super(openFile);
    }
    
    // qnode things
    
    abstract public QNode[] getSources();
    
    public int getSinkCount(){
	return sinkSet.size();
    }
    
    public void addSink(QNode sink){
	sinkSet.add(sink);
    }
    
    public void removeSink(QNode sink){
	sinkSet.remove(sink);
    }
    
    //fixme: future versions will have a more complete assignment/conversion API
    public boolean isAssignableFrom(Class cl) {
	return (this.getClass().isAssignableFrom(cl));
    }
    
    //fixme: is name correct? is assignableTo really opposite to assignableFrom
    public boolean isAssignableTo(Class cl) {
	return (cl.isAssignableFrom(this.getClass()));
    }
    
    //for classes that act as direct references. getValue() will evalute the references.
    public boolean isReference() {
	return false;
    }
    
    //evalute, was getValue()
    public QNodeResource evalute() {
	return this;
    }
    
    //quoted
    public QNodeResource getPrimarySource() {
	return this;
    }
    
    // is this the owner? owners should edit values directly rather than selecting pointers from drop downs
    public boolean isOwner(QNodeResource qnr) {
	if (java.util.Arrays.asList(this.getSources()).contains(qnr)
	    	&& qnr.getSinkCount() == 1) {
	    return true;
	}
	
	return false;
    }
}

