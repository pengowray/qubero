/**
 * ResPointer.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.pointer;

import java.util.HashSet;

import net.pengo.app.OpenFile;
import net.pengo.dependency.QNode;
import net.pengo.resource.QNodeResource;

public class SmartPointer extends QNodeResource // extends QNodeResource
{
    protected QNodeResource value;
    
    //private boolean allowNull; //xxx: implement.. also isFinal, isMutable, isVolatile, etc
    
    public SmartPointer() {
        ResourceRegistry.instance().add(this);
        
    }
    
    public String toString() {
        if (getName()==null)
            return "SmartPointer to " + value;
        
        return getName();
    }
    
    public void setValue(QNodeResource value) {
        if (this.value != null)
            this.value.removeSink(this);
        
        if (value != null)
            value.addSink(this);
        
        this.value = value;
    }
    
    /** note: use getPrimarySource for what is being pointing directly to.
     *  getValue() will "evalute" a pointer (and any pointers it points to) to get a final value 
     */
    
    public QNodeResource evalute() {
        //fixme: check for circular references
        HashSet circularity = new HashSet();
        circularity.add(this);
        QNodeResource c = getPrimarySource();
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
    
    public QNodeResource getPrimarySource() {
        return value;
    }
    
    public QNode[] getSources() {
        return new QNode[]{value};
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

