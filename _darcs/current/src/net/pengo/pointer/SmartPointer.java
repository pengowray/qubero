/**
 * ResPointer.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.pointer;

import java.util.HashSet;

import net.pengo.resource.Resource;
import net.pengo.resource.ResourceAlertListener;

public class SmartPointer extends Resource implements ResourceAlertListener // extends QNodeResource
{
    protected Resource value;
    
    //private boolean allowNull; //xxx: implement.. also isFinal, isMutable, isVolatile, etc
    
    public SmartPointer() {
        ResourceRegistry.instance().add(this);
        
    }
//
//    public String toString() {
//        if (getName()==null)
//            return "SmartPointer to " + value;
//
//        return getName();
//    }
    
    public void setValue(Resource value) {
        if (this.value == value)
            return;
        
        if (this.value != null) {
            this.value.removeSink(this);
	    this.value.removeAlertListenerer(this);
	}
        
        if (value != null) {
            value.addSink(this);
	    value.addAlertListenerer(this);
	}
        
//        if (this.value.equals(value)) {
//            //FIXME: do not alert listeners ? .. should alert SOME type of listener
//            // this really needs contexts to decide
//            this.value = value;
//            return;
//        }
        
        this.value = value;
        alertChangeListenerer();
    }
    
    //propagate changes up to parent
    public void valueChanged(Resource updated) {
	alertChangeListenerer();
    }
    
    public void resourceRemoved(Resource removed) {
	//fixme: something something
    }

    /** note: use getPrimarySource for what is being pointing directly to.
     *  getValue() will "evalute" a pointer (and any pointers it points to) to get a final value
     */
    
    public Resource evaluate() {
        //fixme: check for circular references
        HashSet circularity = new HashSet();
        circularity.add(this);
        Resource c = getPrimarySource();
        
        if (c == null)
        	return null;  //FIXME: is this right?
        
        while (c.isReference()) {
            if (circularity.contains(c)) {
                //circular reference found!
                //fixme: can you spell "error handling"
                System.out.println("circular reference found! argh! abort!!");
                return null;
            }
            circularity.add(c);
            c = c.getPrimarySource();
        }
        return c;
    }
    
    public Resource getPrimarySource() {
        return value;
    }
    
    public Resource[] getSources() {
        return new Resource[]{value};
    }
    
    //fixme: is this ok?
    public boolean isAssignableFrom(Object cl) {
        return false;
    }
    
    public boolean isAssignableTo(Object cl) {
        return false;
    }
    
    public boolean isReference() {
        return true;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.QNodeResource#editProperties()
     */
    public void editProperties() {
        // TODO Auto-generated method stub
    }
}

