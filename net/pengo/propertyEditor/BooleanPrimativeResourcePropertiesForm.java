/*
 * BooleanResourcePropertiesForm.java
 *
 * Created on August 24, 2003, 7:01 AM
 */

package net.pengo.propertyEditor;


import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;
/**
 *
 * @author  Peter Halasz
 *
 */
public class BooleanPrimativeResourcePropertiesForm extends AbstractResourcePropertiesForm {
    private BooleanResource boolRes;
    
    /** Creates a new instance of BooleanResourcePropertiesForm */
    public BooleanPrimativeResourcePropertiesForm(BooleanResource boolRes) {
        super();
        this.boolRes = boolRes;
    }
    
    protected PropertyPage[] getMenu() {
        return new PropertyPage[] {
            new SetBooleanPage(boolRes, this)
        };
    }
    
}
