/*
 * SelectionResource.java
 *
 * Created on 4 September 2002, 19:40
 */

/**
 *
 * @author  Peter Halasz
 */
package net.pengo.resource;
import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.dependency.QNode;

import javax.swing.*;

//acts as a wrapper for LongListSelectionModel.

abstract public class SelectionResource extends QNodeResource implements LongListSelectionListener, QNode {

    
    public SelectionResource(OpenFile openFile) {
        super(openFile);
    }
    
     abstract public LongListSelectionModel getSelection();

     abstract public SelectionData getSelectionData();
     
    /**
     * Called whenever the value of the selection changes.
     * @param e the event that characterizes the change.
     *
     */
    public void valueChanged(LongListSelectionEvent e) {
	if (e.getValueIsAdjusting()) {
	    return;
	}
        
	updated();
    }
    
    abstract public void updated();
    

    

    //FIXME:; reimplement?
    /*
     public boolean equals(SelectionResource o) {
     if (o == this)
     return true;
     
     openFile.getSelectionModel().eq
     return (sel.getStart() == o.sel.getStart() && sel.getLength() == o.sel.getLength());
     }
     
     public boolean equals(Object o) {
     if (o == this)
     return true;
     
     if (o instanceof SelectionResource) {
     return equals((SelectionResource)o);
     }
     
     return false;
     }
     */
        
}
