/*
 * BooleanResourcePropertiesForm.java
 *
 * Created on August 24, 2003, 7:01 AM
 */

package net.pengo.propertyEditor;

import net.pengo.resource.BooleanResource;

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
                new NamePage(boolRes, this),
                new SetBooleanPage(boolRes, this),
                new QNodePage(boolRes)
        };
    }
    
}
